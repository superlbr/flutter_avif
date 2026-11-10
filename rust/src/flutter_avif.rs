use allo_isolate::Isolate;
use async_std::task;
use libavif_sys::avifDecoder;
use protobuf::Message;
use std::collections::HashMap;
use std::mem;
use std::os::raw::c_uchar;
use std::slice;
use std::sync::RwLock;
use std::time::Instant;

use crate::models::avif_info::AvifInfo;
use crate::models::encode_request::EncodeRequest;
use crate::models::frame::Frame;
use crate::models::key_request::KeyRequest;

/// 分段计时开关，三个通道取或，进程内只求值一次（首帧时 cache 进 OnceLock）：
///
/// 1. 编译期 `--features trace-always`：无条件开启，用于属性/环境变量都设不了的设备；
/// 2. 环境变量 `FLUTTER_AVIF_TRACE` 存在即开启（值不看，存在就算）——
///    桌面直接 `FLUTTER_AVIF_TRACE=1 ./x`；Android 需 root 或 debuggable 包 + `wrap.*`；
/// 3. Android 系统属性 `debug.flutter_avif.trace = 1`（也接受 `true`）：
///    `adb shell setprop debug.flutter_avif.trace 1`。
///
/// 注意：窗口只有「进程启动后第一次解码」这一瞬间。属性是那时读一次并缓存的，
/// 之后再 `setprop`/`setprop ... ""` 都不会改变本进程行为——改完必须重启 App，
/// 关掉同样要再重启一次。
///
/// 输出固定一行，单位微秒，字段顺序固定，便于脚本解析：
/// ```text
/// [avif-trace] frame 896x1200 t1=412 t2=7898 t3=595 t4=3510 t5=-1 total=12415
/// ```
/// t1 = create+Parse（header）、t2 = NextImage（dav1d 出 YUV）、t3 = YUVToRGB、
/// t4 = write_to_bytes 组包完成，四个值都是**相对上一段的耗时**；`total` = t0→t4。
/// `t5` 恒为 -1：postCObject 发生在 `allo_isolate::Isolate::task` 内部，从这里量不到，
/// 需要 Dart 侧用「回调到达时间 − t4」补。
fn trace_enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| {
        if cfg!(feature = "trace-always") {
            return true;
        }
        if std::env::var_os("FLUTTER_AVIF_TRACE").is_some() {
            return true;
        }
        #[cfg(target_os = "android")]
        {
            return property_is(b"debug.flutter_avif.trace\0", b"1");
        }
        #[cfg(not(target_os = "android"))]
        {
            false
        }
    })
}

/// 读 Android 系统属性，与 `expected` 相等（大小写不敏感）即为真。
/// `__system_property_get` 返回值的字节数，未设置时返回 0——空值一律当关闭。
#[cfg(target_os = "android")]
fn property_is(name: &[u8], expected: &[u8]) -> bool {
    extern "C" {
        fn __system_property_get(name: *const c_uchar, value: *mut c_uchar) -> i32;
    }
    // PROP_VALUE_MAX = 92，含结尾 NUL
    let mut buf = [0u8; 92];
    let len = unsafe { __system_property_get(name.as_ptr(), buf.as_mut_ptr()) };
    if len <= 0 || len as usize >= buf.len() {
        return false;
    }
    buf[..len as usize].eq_ignore_ascii_case(expected)
}

struct Trace {
    on: bool,
    t0: Instant,
    t1: Option<Instant>,
    t2: Option<Instant>,
    t3: Option<Instant>,
    t4: Option<Instant>,
}

impl Trace {
    #[inline]
    fn new() -> Self {
        Trace {
            on: trace_enabled(),
            t0: Instant::now(),
            t1: None,
            t2: None,
            t3: None,
            t4: None,
        }
    }

    #[inline]
    fn mark1(&mut self) {
        if self.on {
            self.t1 = Some(Instant::now());
        }
    }

    #[inline]
    fn mark2(&mut self) {
        if self.on {
            self.t2 = Some(Instant::now());
        }
    }

    #[inline]
    fn mark3(&mut self) {
        if self.on {
            self.t3 = Some(Instant::now());
        }
    }

    #[inline]
    fn mark4(&mut self) {
        if self.on {
            self.t4 = Some(Instant::now());
        }
    }

    fn emit(&self, width: u32, height: u32) {
        if !self.on {
            return;
        }

        let us = |a: Instant, b: Instant| b.saturating_duration_since(a).as_micros();
        let t1 = self.t1.unwrap_or(self.t0);
        let t2 = self.t2.unwrap_or(t1);
        let t3 = self.t3.unwrap_or(t2);
        let t4 = self.t4.unwrap_or(t3);

        let line = format!(
            "[avif-trace] frame {}x{} t1={} t2={} t3={} t4={} t5=-1 total={}",
            width,
            height,
            us(self.t0, t1),
            us(t1, t2),
            us(t2, t3),
            us(t3, t4),
            us(self.t0, t4),
        );
        log_line(&line);
    }
}

#[cfg(target_os = "android")]
fn log_line(msg: &str) {
    extern "C" {
        fn __android_log_print(prio: i32, tag: *const c_uchar, fmt: *const c_uchar, ...) -> i32;
    }
    // ANDROID_LOG_INFO = 4
    static TAG: &[u8] = b"avif-trace\0";
    static FMT: &[u8] = b"%s\0";
    let mut buf = Vec::with_capacity(msg.len() + 1);
    buf.extend_from_slice(msg.as_bytes());
    buf.push(0);
    unsafe {
        __android_log_print(4, TAG.as_ptr(), FMT.as_ptr(), buf.as_ptr());
    }
}

#[cfg(not(target_os = "android"))]
fn log_line(msg: &str) {
    println!("{}", msg);
}

/// 改动 4：dav1d 的解码线程数，**默认关闭的可选开关**。
///
/// 上游只给 encoder 设了 `maxThreads`，decoder 用的是 libavif 默认值 1。实测结论：
///   * 多 tile 图（avifenc 默认 autotiling 出的 2x4）4 线程约 3.1~3.5x；
///   * 单 tile 图（当前线上形态，1x1）只有 1.02x，等于没有收益，还会多出几个
///     约 3.8MB 的 YUV 槽位，内存峰值反而变差。
///
/// 所以这里不硬编码，做成 cargo feature：需要时用
///   `./build.sh --features max-threads`
/// 打开，默认构建保持上游的串行解码行为。
#[cfg(feature = "max-threads")]
const DECODE_MAX_THREADS: i32 = 4;

lazy_static::lazy_static! {
    static ref DECODERS: RwLock<HashMap<String, DecoderRef>> = {
        RwLock::new(HashMap::new())
    };
}

#[no_mangle]
pub extern "C" fn decode_single_frame_image(port: i64, ptr: *const c_uchar, len: usize) -> i64 {
    let pb_bytes = unsafe { slice::from_raw_parts(ptr, len) };
    let input = KeyRequest::parse_from_bytes(&Vec::from(pb_bytes)).unwrap();
    let isolate = Isolate::new(port);

    task::spawn(isolate.task(async move {
        let mut trace = Trace::new();
        let decoder = _init_memory_decoder(input.data.as_ptr(), input.data.len(), &mut trace);
        let image = _get_next_frame(decoder, &mut trace);
        _dispose_decoder(decoder);

        let (width, height) = (image.width, image.height);
        let bytes = image.write_to_bytes().unwrap();
        trace.mark4();
        trace.emit(width, height);

        return bytes;
    }));

    return port;
}

#[no_mangle]
pub extern "C" fn init_memory_decoder(port: i64, ptr: *const c_uchar, len: usize) -> i64 {
    let pb_bytes = unsafe { slice::from_raw_parts(ptr, len) };
    let input = KeyRequest::parse_from_bytes(&Vec::from(pb_bytes)).unwrap();
    let isolate = Isolate::new(port);

    {
        let map = DECODERS.read().unwrap();
        if map.contains_key(&input.key) {
            let decoder = &map[&input.key];

            let output = decoder.info.write_to_bytes().unwrap();

            isolate.post(output);

            return port;
        }
    }

    task::spawn(isolate.task(async move {
        unsafe {
            let mut avif_bytes = input.data;
            let mut trace = Trace::new();
            let decoder =
                _init_memory_decoder(avif_bytes.as_ptr(), avif_bytes.len(), &mut trace);

            let mut avif_info = AvifInfo::new();
            avif_info.width = 0;
            avif_info.height = 0;
            avif_info.duration = (*decoder).duration;
            avif_info.image_count = (*decoder).imageCount as u32;

            let mut map = DECODERS.write().unwrap();
            map.insert(
                input.key,
                DecoderRef {
                    decoder: decoder,
                    ptr: avif_bytes.as_mut_ptr(),
                    len: avif_bytes.len(),
                    info: avif_info.clone(),
                },
            );
            mem::forget(avif_bytes);

            return avif_info.write_to_bytes().unwrap();
        }
    }));

    return port;
}

#[no_mangle]
pub extern "C" fn reset_decoder(port: i64, ptr: *const c_uchar, len: usize) -> i64 {
    let pb_bytes = unsafe { slice::from_raw_parts(ptr, len) };
    let input = KeyRequest::parse_from_bytes(&Vec::from(pb_bytes)).unwrap();
    let isolate = Isolate::new(port);

    task::spawn(isolate.task(async move {
        {
            let map = DECODERS.read().unwrap();
            if !map.contains_key(&input.key) {
                return false;
            }

            let decoder = &map[&input.key];
            _reset_decoder(decoder.decoder);
        }

        return true;
    }));

    return port;
}

#[no_mangle]
pub extern "C" fn dispose_decoder(port: i64, ptr: *const c_uchar, len: usize) -> i64 {
    let pb_bytes = unsafe { slice::from_raw_parts(ptr, len) };
    let input = KeyRequest::parse_from_bytes(&Vec::from(pb_bytes)).unwrap();
    let isolate = Isolate::new(port);

    task::spawn(isolate.task(async move {
        {
            let mut map = DECODERS.write().unwrap();
            if !map.contains_key(&input.key) {
                return false;
            }

            let decoder = &map[&input.key];
            _dispose_decoder(decoder.decoder);

            drop(unsafe { Vec::from_raw_parts(decoder.ptr, decoder.len, decoder.len) });
            map.remove(&input.key);
        }

        return true;
    }));

    return port;
}

#[no_mangle]
pub extern "C" fn get_next_frame(port: i64, ptr: *const c_uchar, len: usize) -> i64 {
    let pb_bytes = unsafe { slice::from_raw_parts(ptr, len) };
    let input = KeyRequest::parse_from_bytes(&Vec::from(pb_bytes)).unwrap();
    let isolate = Isolate::new(port);

    task::spawn(isolate.task(async move {
        {
            let map = DECODERS.read().unwrap();
            if !map.contains_key(&input.key) {
                panic!("Decoder not found. {}", input.key);
            }

            let decoder = &map[&input.key];
            let mut trace = Trace::new();
            let result = _get_next_frame(decoder.decoder, &mut trace);

            let (width, height) = (result.width, result.height);
            let bytes = result.write_to_bytes().unwrap();
            trace.mark4();
            trace.emit(width, height);

            return bytes;
        }
    }));

    return port;
}

#[no_mangle]
pub extern "C" fn encode_avif(port: i64, ptr: *const c_uchar, len: usize) -> i64 {
    let pb_bytes = unsafe { slice::from_raw_parts(ptr, len) };
    let input = EncodeRequest::parse_from_bytes(&Vec::from(pb_bytes)).unwrap();
    let isolate = Isolate::new(port);

    task::spawn(isolate.task(async move {
        unsafe {
            let encoder = libavif_sys::avifEncoderCreate();
            (*encoder).maxThreads = input.max_threads;
            (*encoder).speed = input.speed;
            (*encoder).timescale = u64::from(input.timescale);
            (*encoder).minQuantizer = input.min_quantizer;
            (*encoder).maxQuantizer = input.max_quantizer;
            (*encoder).minQuantizerAlpha = input.min_quantizer_alpha;
            (*encoder).maxQuantizerAlpha = input.max_quantizer_alpha;

            let image_sequence = input.image_list;

            for frame in image_sequence.iter() {
                let image = libavif_sys::avifImageCreate(
                    input.width,
                    input.height,
                    8,
                    libavif_sys::AVIF_PIXEL_FORMAT_YUV444,
                );
                libavif_sys::avifImageAllocatePlanes(image, libavif_sys::AVIF_PLANES_YUV);

                let mut rgb = libavif_sys::avifRGBImage::default();
                let raw_rgb = &mut rgb as *mut libavif_sys::avifRGBImage;
                libavif_sys::avifRGBImageSetDefaults(raw_rgb, image);
                rgb.format = libavif_sys::AVIF_RGB_FORMAT_RGBA;
                rgb.depth = 8;
                libavif_sys::avifRGBImageAllocatePixels(raw_rgb);

                std::ptr::copy(
                    frame.data.as_ptr(),
                    rgb.pixels,
                    (rgb.rowBytes * (*image).height) as usize,
                );

                if input.exif_data.len() > 0 {
                    libavif_sys::avifImageSetMetadataExif(
                        image,
                        input.exif_data.as_ptr(),
                        input.exif_data.len(),
                    );
                }

                let conversion_result = libavif_sys::avifImageRGBToYUV(image, &rgb);
                if conversion_result != libavif_sys::AVIF_RESULT_OK {
                    libavif_sys::avifImageDestroy(image);
                    libavif_sys::avifEncoderDestroy(encoder);
                    libavif_sys::avifRGBImageFreePixels(raw_rgb);
                    panic!("yuv_to_rgb error {}", conversion_result);
                }
                let add_result = libavif_sys::avifEncoderAddImage(
                    encoder,
                    image,
                    u64::from(frame.duration_in_timescale),
                    libavif_sys::AVIF_ADD_IMAGE_FLAG_NONE,
                );
                if add_result != libavif_sys::AVIF_RESULT_OK {
                    libavif_sys::avifImageDestroy(image);
                    libavif_sys::avifEncoderDestroy(encoder);
                    libavif_sys::avifRGBImageFreePixels(raw_rgb);
                    panic!("add_image error {}", add_result);
                }
                libavif_sys::avifImageDestroy(image);
                libavif_sys::avifRGBImageFreePixels(raw_rgb);
            }

            let mut s = ::std::mem::MaybeUninit::<u8>::uninit();
            let mut avif_output = libavif_sys::avifRWData {
                data: s.as_mut_ptr(),
                size: 0,
            };
            let raw_avif_output = &mut avif_output as *mut libavif_sys::avifRWData;
            let finish_result = libavif_sys::avifEncoderFinish(encoder, raw_avif_output);
            if finish_result != libavif_sys::AVIF_RESULT_OK {
                libavif_sys::avifRWDataFree(raw_avif_output);
                libavif_sys::avifEncoderDestroy(encoder);
                panic!("avif_output error {}", finish_result);
            }
            let output_data = slice::from_raw_parts(avif_output.data, avif_output.size).to_vec();
            libavif_sys::avifRWDataFree(raw_avif_output);
            libavif_sys::avifEncoderDestroy(encoder);

            return output_data;
        }
    }));

    return port;
}

fn _init_memory_decoder(ptr: *const u8, len: usize, trace: &mut Trace) -> *mut avifDecoder {
    unsafe {
        let decoder = libavif_sys::avifDecoderCreate();
        // 改动 4（可选）：必须在 Parse 之前设置，libavif 会按此创建 dav1d 线程池
        #[cfg(feature = "max-threads")]
        {
            (*decoder).maxThreads = DECODE_MAX_THREADS;
        }

        let set_memory_result = libavif_sys::avifDecoderSetIOMemory(decoder, ptr, len);
        if set_memory_result != libavif_sys::AVIF_RESULT_OK {
            libavif_sys::avifDecoderDestroy(decoder);
            panic!("Couldn't decode the image. Code: {}", set_memory_result);
        }

        let parse_result = libavif_sys::avifDecoderParse(decoder);
        if !(parse_result == libavif_sys::AVIF_RESULT_OK
            || parse_result == libavif_sys::AVIF_RESULT_BMFF_PARSE_FAILED)
        {
            libavif_sys::avifDecoderDestroy(decoder);
            panic!("Couldn't decode the image. Code: {}", parse_result);
        }

        trace.mark1();

        return decoder;
    }
}

fn _dispose_decoder(decoder: *mut libavif_sys::avifDecoder) -> Frame {
    unsafe {
        libavif_sys::avifDecoderDestroy(decoder);
    }

    let mut frame = Frame::new();
    frame.data = Vec::new();
    frame.duration = 0.0;
    frame.width = 0;
    frame.height = 0;

    return frame;
}

fn _reset_decoder(decoder: *mut libavif_sys::avifDecoder) -> Frame {
    unsafe {
        libavif_sys::avifDecoderReset(decoder);
    }

    let mut frame = Frame::new();
    frame.data = Vec::new();
    frame.duration = 0.0;
    frame.width = 0;
    frame.height = 0;

    return frame;
}

// rgb 的若干个字段是写给 C 侧读的（经 raw_rgb 指针），rustc 看不到这次读取，
// 会报 `unused_assignments`，属于误报。
#[allow(unused_assignments)]
fn _get_next_frame(decoder: *mut libavif_sys::avifDecoder, trace: &mut Trace) -> Frame {
    unsafe {
        let mut decode_result = libavif_sys::avifDecoderNextImage(decoder);
        if decode_result == libavif_sys::AVIF_RESULT_NO_IMAGES_REMAINING {
            libavif_sys::avifDecoderReset(decoder);
            decode_result = libavif_sys::avifDecoderNextImage(decoder);
        }

        if decode_result != libavif_sys::AVIF_RESULT_OK {
            panic!("decode error {}", decode_result);
        }
        trace.mark2();

        let image = (*decoder).image;
        let width = (*image).width;
        let height = (*image).height;
        // RGBA8888，每像素 4 字节
        let row_bytes = width * 4;
        let size = (row_bytes as usize) * (height as usize);

        let mut rgb = libavif_sys::avifRGBImage::default();
        let raw_rgb = &mut rgb as *mut libavif_sys::avifRGBImage;

        libavif_sys::avifRGBImageSetDefaults(raw_rgb, image);
        rgb.format = libavif_sys::AVIF_RGB_FORMAT_RGBA;
        rgb.depth = 8;
        rgb.alphaPremultiplied = libavif_sys::AVIF_TRUE as i32;

        // 改动 1（本仓库最关键的改动）：
        // 上游这里是 avifRGBImageAllocatePixels() 让 libavif 自己 malloc 一块 RGBA，
        // 转换完再 slice::from_raw_parts(...).to_vec() 整帧拷一份 —— 单帧峰值同时存在
        // 两块 10.1MB（1080x2346）的 RGBA，多一次全量 memcpy。
        //
        // 现在直接把这个 Vec 的指针交给 libavif，让 avifImageYUVToRGB 就地写入，
        // 然后把 Vec move 进 Frame。少一次整帧拷贝，也少一块并存的整帧缓冲。
        let mut data: Vec<u8> = vec![0u8; size];
        rgb.pixels = data.as_mut_ptr();
        rgb.rowBytes = row_bytes;

        let conversion_result = libavif_sys::avifImageYUVToRGB(image, raw_rgb);
        if conversion_result != libavif_sys::AVIF_RESULT_OK {
            panic!("yuv_to_rgb error {}", conversion_result);
        }
        trace.mark3();
        // avifRGBImageFreePixels 不需要调用：像素内存归上面的 Vec 所有。

        let mut frame = Frame::new();
        frame.data = data;
        frame.duration = (*decoder).imageTiming.duration;
        frame.width = width;
        frame.height = height;

        return frame;
    }
}

struct DecoderRef {
    decoder: *mut avifDecoder,
    ptr: *mut u8,
    len: usize,
    info: AvifInfo,
}

unsafe impl Send for DecoderRef {}
unsafe impl Sync for DecoderRef {}
