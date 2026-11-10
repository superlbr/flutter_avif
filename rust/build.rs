// 显式把 libavif.a / libdav1d.a 交给链接器。
//
// 为什么需要：开 LTO 后 rustc 的 fat-LTO 链接路径会把依赖 build script 里的
// `cargo:rustc-link-lib=static=avif|dav1d` 丢掉，结果 cdylib 链接时**不报错**
// （共享库允许未定义符号），但产物里 avifDecoderCreate / dav1d_* 全是 UND，
// 体积也从 ~2.6MB 缩到 862KB，装到设备上必然加载失败。
//
// 用 `cargo:rustc-link-arg` 直接把这几个归档文件塞进链接行，绕开被丢弃的那条路径，
// 同时用 --start-group/--end-group 解决 libavif <-> libdav1d 的相互引用顺序问题。

use std::env;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");

    let avif = env::var("DEP_AVIF_STATICLIB")
        .expect("DEP_AVIF_STATICLIB 缺失：确认 vendors/libavif-sys 的 Cargo.toml 有 links = \"avif\"");
    let dav1d = env::var("DEP_DAV1D_STATICLIB")
        .expect("DEP_DAV1D_STATICLIB 缺失：确认 libdav1d-sys 在 build-dependencies 里");

    for p in [&avif, &dav1d] {
        if !std::path::Path::new(p).exists() {
            panic!("静态库不存在: {p}（build script 可能被缓存跳过，试 cargo clean）");
        }
    }

    println!("cargo:rustc-link-arg=-Wl,--start-group");
    println!("cargo:rustc-link-arg={avif}");
    println!("cargo:rustc-link-arg={dav1d}");
    println!("cargo:rustc-link-arg=-Wl,--end-group");
    // libavif 用到 libm，Android 上不会自动带上
    println!("cargo:rustc-link-arg=-lm");
}
