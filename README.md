# flutter_avif（自编 Android `.so` 单仓库版）

把 [yekeskin/flutter_avif](https://github.com/yekeskin/flutter_avif) 3.1.0 的**插件本体**、
**Rust FFI crate**、以及它依赖的两个 `-sys` crate 的**源码**全部放在同一个仓库里，
改掉解码路径上的几处冗余后，由 CI 或本机 NDK 直接产出 `libflutter_avif.so`。

- 上游版本：`flutter_avif 3.1.0` / `flutter_avif_android 3.1.0` / `rust` crate 同步自
  upstream `main` 分支
- 依赖版本：`libavif 1.0.4`（libavif-sys 0.17.0）、`dav1d 1.4.3`（libdav1d-sys 0.7.1，
  这是 crates.io 上最新的**带源码**版本）、`libyuv 1974`（vendored，YUV→RGBA 快路径）
- 依赖不 fork：`rust/Cargo.toml` 用 `[patch.crates-io]` 指向本仓库 `vendors/`，
  `git clone` 后一条 `./build.sh` 就能编

## 仓库结构

```
flutter_avif/                     Dart 侧 API（path 依赖下面三个包）
flutter_avif_platform_interface/  FFI 绑定 + protobuf 模型
flutter_avif_android/             Android 实现；jniLibs/ 里是编好的 .so
flutter_avif_ios/                 iOS 实现（未改动，podspec 会解压 xcframework.zip）
rust/                             主 crate（cdylib = libflutter_avif.so）
vendors/libavif-sys/              libavif 1.0.4 源码 + cmake 构建脚本
vendors/libdav1d-sys/             dav1d 1.4.3 源码（vendor/）+ meson 构建脚本
vendors/libyuv/                   libyuv 1974 源码（只做 YUV→RGB，不编 jpeg）
bench/                            宿主上的 reformat 单点 bench（不参与 Android 构建）
cmake/android-toolchain.cmake     NDK toolchain 包装（见「构建陷阱」）
build.sh                          一条命令出两个 ABI 的 .so
.github/workflows/build-android.yml
```

## 构建

```bash
./build.sh                          # arm64-v8a + armeabi-v7a
./build.sh arm64-v8a                # 只编一个
./build.sh --features max-threads   # 额外打开解码多线程（见改动 4）
```

产物会同时写到 `dist/<abi>/libflutter_avif.so` 和
`flutter_avif_android/android/src/main/jniLibs/<abi>/libflutter_avif.so`。

前置条件（CI 里都已写好）：

| 依赖 | 版本 | 说明 |
|---|---|---|
| rustc / cargo | ≥ 1.75（本仓库验证 1.96.0） | 需 `rustup target add aarch64-linux-android armv7-linux-androideabi` |
| cargo-ndk | 4.1.2 | `cargo install cargo-ndk --version 4.1.2 --locked` |
| meson / ninja | meson ≥ 1.3 | dav1d 用 meson 编译 |
| Android NDK | 28.2.13676358（r28c） | `build.sh` 会自动从 `ANDROID_NDK_HOME` / SDK 目录找 |

## 五处改动

### 改动 1：去掉 Rust 侧两次整帧 RGBA 拷贝（最关键）

上游 `_get_next_frame` 是：

```
avifRGBImageAllocatePixels()   → libavif 自己 malloc 一块 RGBA
slice::from_raw_parts(...).to_vec()  → 整帧 memcpy 到 Vec
avifRGBImageFreePixels()       → 再把那块释放掉
```

单帧 1080×2346 RGBA 是 10.13MB，这段代码在拷完之前**两块 10.13MB 同时存在**，
峰值比必要值高约 10MB，并且白白多做一次全量 memcpy。

现在把 `avifRGBImage.pixels` 直接指向 `Vec<u8>` 的缓冲区，让 `avifImageYUVToRGB`
就地写入，然后把这个 `Vec` move 进 `Frame`：

```rust
let mut data: Vec<u8> = vec![0u8; size];
rgb.pixels = data.as_mut_ptr();
rgb.rowBytes = row_bytes;
libavif_sys::avifImageYUVToRGB(image, raw_rgb);
frame.data = data;
```

- **少一块整帧缓冲**：单页峰值 −10.13MB（1080×2346）
- **少一次整帧 memcpy**：约 10MB 的内存带宽
- 说明：上游用的是 `allo_isolate` 的 protobuf 通道，不是 flutter_rust_bridge，
  所以 `ZeroCopyBuffer` 那条路走不通；「让 libavif 写进预分配缓冲」是等价且更彻底的做法。

### 改动 2：dav1d 1.0.0 → 1.4.3

上游 `libavif-sys 0.17.0` 锁的是 `libdav1d-sys 0.7.1`，而 `0.7.1` 内部自带的是
**dav1d 1.4.3**（不是 1.0.0）；crates.io 上没有带 1.5.x 源码的版本，所以落到 1.4.3。

相对上游旧产物里的 dav1d 1.0.0：**实测**单线程解码一张细节丰富的 1080 页快 −22.8%，
但平坦/低码率的页面只有 −4%~−6%。

### 改动 3：删 rav1e，开 LTO / codegen-units=1 / strip

- `vendors/libavif-sys/Cargo.toml` 删掉 `rav1e` 依赖与 `codec-rav1e` feature，
  `build.rs` / `src/lib.rs` 里对应分支一并删除 —— 本仓库只做**解码**，
  编码入口 `encode_avif` 虽然还在（Dart 侧 API 保持不变），但编码器走的是 libavif 的
  默认 AOM/dav1d 能力，不含 rav1e。上游 rav1e 在 `.text` 里占 ≥392KB（实测）。
- `rust/Cargo.toml` 的 `[profile.release]`：`opt-level=3`、`lto=true`、
  `codegen-units=1`、`strip="debuginfo"`，`build.sh` 最后再跑一遍 `llvm-strip --strip-unneeded`。
- `panic="unwind"` 不要改成 `abort`：上游 `unwrap()` 很多，abort 会把异步任务里的 panic
  升级成整个进程崩溃。

### 改动 4：decoder 多线程 —— 只留可选开关，默认关闭

上游只给 encoder 设了 `maxThreads`，decoder 一直是 libavif 的默认值 1。实测：

- 多 tile 图（avifenc 默认 autotiling 出的 2×4）：4 线程 **3.1~3.5×**
- 单 tile 图（**当前线上形态就是 1×1 单 tile**）：只有 **1.02×**，等于没收益，
  还会多出几个约 3.8MB 的 YUV 槽位，内存峰值反而变差

所以这条不硬编码，做成 cargo feature `max-threads`（默认关）：

```bash
./build.sh --features max-threads
```

### 改动 5：打开 libyuv，YUV→RGBA 改走 SIMD

上游 `libavif-sys` 的构建脚本里写着 `-DCMAKE_DISABLE_FIND_PACKAGE_libyuv=1`，所以
`avifImageYUVToRGB` 一直跑的是 libavif `src/reformat.c` 的**标量 C 实现**，这一块在
1080×2346 单页上要 19~20ms（宿主）/ 22~24ms（真机），是解码总时长里最大的一块固定开销。

现在把 `vendors/libyuv`（版本 **1974**）一起编成静态库，并把路径直接灌进 libavif 的
cache 变量，让它走 `src/reformat_libyuv.c`：

```rust
avif.define("LIBYUV_INCLUDE_DIR", &yuv_include);
avif.define("LIBYUV_LIBRARY", &yuv_staticlib);
avif.define("CMAKE_DISABLE_FIND_PACKAGE_libyuv", "0"); // 老 CMakeCache 里还留着 =1
```

几个必须注意的点：

- **版本要 ≥1813**。libavif 里是硬编码的门槛：`<1755` 直接禁用快路径；`<1813` 会
  `#define I420ToARGBMatrixFilter NULL` 等一批 `Filter` 变体（bilinear 相关），并在
  configure 时打 `libavif: some libyuv optimizations require at least version 1813 to work.`。
  本仓库用 1974，configure 输出是
  `libavif: libyuv (1974) found; libyuv-based fast paths enabled.`，没有那条降级警告。
- **`.a` 要 PIC**：静态库最终进 `.so`，`CMAKE_POSITION_INDEPENDENT_CODE=ON` 不能漏。
- **链接组里要带上它**：`reformat_libyuv.c` 引用 yuv 符号，`rust/build.rs` 除了
  `libavif.a` / `libdav1d.a` 之外，把 `libyuv.a` 也放进 `--start-group ... --end-group`。
- **验证符号真的链进去了**，而不是只看 cmake 成功（strip 掉之后 `.so` 里就查不到了，
  要在 `rust/target/<triple>/release/libflutter_avif.so` 上查）：

```bash
llvm-nm --defined-only rust/target/aarch64-linux-android/release/libflutter_avif.so \
  | grep -E "I420ToARGBMatrix|I400ToARGBMatrix"   # 均为 t
llvm-nm -u rust/target/aarch64-linux-android/release/libflutter_avif.so \
  | grep -cE "I420To|ARGBMatrix"                  # 0
```

`NV12ToARGBMatrix` / `J420ToARGB` 查不到是**正常**的：AVIF 解出来是 3 平面 YUV420
（不是 NV12），range 则统一用 `I*ToARGBMatrix` + 对应 `YuvConstants` 表达，不走 J 系列。

#### 实测（宿主 Apple Silicon，n=50，只量 `avifImageYUVToRGB` 本身）

`bench/run_bench.sh` 会编标量/libyuv 两份 libavif，同一份 YUV 平面各跑 50 次：

| 样本 | 尺寸 | 标量 C | libyuv | 倍数 |
|---|---|---|---|---|
| 细节页 page_default | 1080×2346 | 20.42 ms | **0.78 ms** | 26.2× |
| 中间页 mid | 1080×2346 | 19.70 ms | **0.79 ms** | 24.9× |
| 平坦页 flat | 1080×2346 | 19.23 ms | **0.79 ms** | 24.3× |
| 阅读器宽 585 | 585×1271 | 5.56 ms | **0.25 ms** | 22.2× |
| 阅读器宽 585 | 585×2346 | 11.53 ms | **0.48 ms** | 24.0× |

> 「2~3×」是流传的说法，不是 libavif 源码或 CMake 里写过的承诺；上表是本仓库实测。

`reformat` 落到 ~0.8ms 之后，按行条带并行的收益上限被压到亚毫秒级（线程唤醒/同步
本身就要 0.1~0.3ms），所以**没有做线程化**，`max-threads` 保持默认关闭。

#### 像素差（同图、两条路径的 RGBA 逐字节对比）

| 样本 | 最大差值 | 有差异字节占比 | 差 >1 的字节 |
|---|---|---|---|
| page_default | 2（只在 B 通道，522 字节 / 10,134,720） | 25.64% | 0.005% |
| mid | 1 | 0.01% | 0 |
| flat | 0 | 0% | 0 |

差异来自 libyuv 的定点舍入和 limited/full range 处理，量级是 ±1/255（0.4%），
肉眼不可见；alpha 通道完全一致（PTA 页面没有 alpha，R/G/B 之外无差异）。

## 体积

对照对象是现网 `flutter_avif_android/.../jniLibs/` 里的旧产物（dav1d 1.0.0 + rav1e + 未改解码路径）：

| ABI | 旧（raw） | 新（raw） | 新（strip 后） | 旧（strip 后） | strip 后差值 |
|---|---|---|---|---|---|
| arm64-v8a | 4,362,296 | 1,921,936 | **1,526,112** | 3,309,496 | −1,783,384（−53.9%） |
| armeabi-v7a | 3,061,960 | 1,616,472 | **1,245,132** | 2,405,044 | −1,159,912（−48.2%） |

其中改动 5（libyuv）的代价是：arm64 +156,288（未开 libyuv 时 strip 后 1,369,824，
即 +11.4%）、v7a +102,176（1,142,956 → 1,245,132，+8.9%）。

（旧产物的 strip 后数字 = 原文件减去 24.1% / 21.5% 的 strip 收益，与本次同样口径。）

体积下降主要来自三处：删 rav1e、`--gc-sections` + LTO 把未使用的解码分支收掉、
以及 dav1d 汇编的裁剪。改完的 `.so` 里 `avif*` / `dav1d_*` 全部是本地符号（`t`），
不进动态符号表，只导出 6 个 `#[no_mangle]` 的 Dart FFI 入口。

## 构建陷阱（改这个仓库前务必看）

两个问题都会让构建「成功」但产物不可用，且**不报错**：

1. **CMake 会退回宿主模式**。`libavif-sys` 通过 `cmake` crate 编译，而 `cmake 0.1.48`
   只转发 `CMAKE_TOOLCHAIN_FILE` 环境变量，没法传 `-DANDROID_ABI` / `-DANDROID_PLATFORM`。
   直接指向 NDK 官方 toolchain 时这两个变量缺失，CMake 会按 macOS 判断平台，
   给 clang 追加 `-arch arm64 -isysroot`，报 `unsupported option '-arch'`。
   → 用 `cmake/android-toolchain.cmake` 先从环境变量补齐再 include 官方文件。

2. **dav1d 会编成宿主架构的静态库**。`libdav1d-sys` 上游只给 Windows/Linux/wasm 准备了
   meson cross file，**没有 Android**。缺 cross file 时 meson 用宿主 clang 编出 Mach-O 的
   `libdav1d.a`（符号是 `_dav1d_open`）。这个 `.a` 链接进 ELF 的 `.so` 时被 lld 静默跳过，
   而共享库允许未定义符号，所以链接不报错、产物只有 862KB、`dav1d_*` 全是 `UND`，
   装到设备上 `dlopen` 才会失败。
   → `vendors/libdav1d-sys/build.rs` 里按 NDK 的 `<arch><api>-clang` 现生成 cross file。

3. **开了 LTO 之后必须显式把静态库交给链接器**。fat LTO 的链接路径会丢掉依赖 build script
   的 `cargo:rustc-link-lib=static=avif|dav1d`（`lto=false` 时这条才在）。
   → `rust/build.rs` 通过 `DEP_AVIF_STATICLIB` / `DEP_DAV1D_STATICLIB` / `DEP_AVIF_YUV_STATICLIB`
   （配合 `vendors/libavif-sys` 新增的 `links = "avif"` + `cargo:staticlib=` / `cargo:yuv_staticlib=`）
   拿到三个 `.a` 的绝对路径，用 `cargo:rustc-link-arg` + `--start-group/--end-group`
   显式塞进链接行（`libavif` / `dav1d` / `libyuv` 互相引用，必须成组）。

`build.sh` 最后会用 `llvm-nm -D -u` 检查 `dav1d_*` / `avif*` 是否还有未定义符号，
有就直接报错退出 —— 这三条陷阱的共同特征是「链接成功但不可用」，
所以这道自检不能省。

## CI

`.github/workflows/build-android.yml`：ubuntu-latest + 矩阵（两个 ABI），
装 rust targets、cargo-ndk 4.1.2、meson/ninja、NDK 28.2.13676358，
跑 `./build.sh <abi>`，把 `dist/<abi>/libflutter_avif.so` 作为 artifact 上传；
打 `v*` tag 时额外把两个 `.so` 挂到 GitHub Release。

## 与本产品仓库的关系

本仓库是独立仓库，**没有**改动、也没有向 `/Users/baorong/Developer/comic` 提交任何内容。
`flutter_avif*` 四个包的源码来自该产品仓库里的同名副本（= 上游 3.1.0 + 三个构建补丁：
删重复的 Kotlin `FlutterAvifPlugin.kt`、`compileOptions` 11→17、`compileSdkVersion` 31→36），
`jniLibs/` 下的 `.so` 已替换为本仓库构建的产物。
