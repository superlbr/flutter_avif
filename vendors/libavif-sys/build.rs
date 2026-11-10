use std::env;
use std::path::Path;

use cmake::Config;

fn main() {
    if env::var_os("DOCS_RS").is_some() {
        return;
    }

    let out_dir_ = env::var("OUT_DIR").unwrap();
    let out_dir = Path::new(&out_dir_);
    let mut avif = Config::new("libavif");

    #[allow(unused_mut)]
    let mut pc_paths: Vec<_> = env::var("PKG_CONFIG_PATH")
        .map(|v| env::split_paths(&v).collect())
        .unwrap_or_default();

    avif.define("BUILD_SHARED_LIBS", "0");
    // Required for clang 12 on macOS, and likely all future compilers libavif hasn't been tweaked for yet
    avif.define("AVIF_ENABLE_WERROR", "0");

    if env::var_os("CI").is_some() {
        avif.very_verbose(true);
    }

    #[cfg(feature = "codec-aom")]
    {
        let include =
            env::var_os("DEP_AOM_INCLUDE").expect("libaom-sys should have set include path");
        avif.define("AVIF_CODEC_AOM", "1");
        avif.define("AOM_INCLUDE_DIR", include);

        let pc_path =
            env::var_os("DEP_AOM_PKGCONFIG").expect("libaom-sys should have set pkgconfig path");
        avif.define("AOM_LIBRARY", pc_path.clone());
        pc_paths.insert(0, pc_path.into());
    }

    // 本地 vendor 时删除 rav1e 分支：只保留 dav1d 解码，见 README「改动 3」。

    #[cfg(feature = "codec-dav1d")]
    {
        let include =
            env::var_os("DEP_DAV1D_INCLUDE").expect("libdav1d-sys should have set pkgconfig path");
        avif.define("AVIF_CODEC_DAV1D", "1");
        avif.define("DAV1D_INCLUDE_DIR", include);

        if let Some(pc_path) = env::var_os("DEP_DAV1D_PKGCONFIG") {
            pc_paths.insert(0, pc_path.into());
        }

        if let Some(staticlib) = env::var_os("DEP_DAV1D_STATICLIB") {
            avif.define("DAV1D_LIBRARY", staticlib);
        }
    }

    eprintln!("building libyuv");
    // 本地 vendor 时新增（改动 5）：上游默认 -DCMAKE_DISABLE_FIND_PACKAGE_libyuv=1，
    // 于是 avifImageYUVToRGB 只能跑 reformat.c 里的标量 C 实现。这里先把仓库内
    // vendors/libyuv（版本 1974）编成静态库，再把路径直接灌进 libavif 的 cache 变量，
    // 让它走 src/reformat_libyuv.c 的 SIMD 快路径。
    let libyuv_src = Path::new(&env::var("CARGO_MANIFEST_DIR").unwrap()).join("../libyuv");
    let mut yuv = Config::new(&libyuv_src);
    yuv.define("BUILD_SHARED_LIBS", "0")
        .define("LIBYUV_DISABLE_JPEG", "ON") // 只要 YUV/RGB 转换，不引 libjpeg
        .define("UNIT_TEST", "OFF")
        .define("CMAKE_POSITION_INDEPENDENT_CODE", "ON") // 静态库要进 .so，必须 PIC
        .profile(if env::var("PROFILE").expect("PROFILE") == "release" {
            "Release"
        } else {
            "Debug"
        });
    let yuv_built = yuv.build();
    let yuv_include = yuv_built.join("include");
    let yuv_staticlib = yuv_built.join("lib").join("libyuv.a");
    assert!(
        yuv_staticlib.exists(),
        "libyuv 没编出来：{}",
        yuv_staticlib.display()
    );

    avif.define("LIBYUV_INCLUDE_DIR", &yuv_include);
    avif.define("LIBYUV_LIBRARY", &yuv_staticlib);
    // 显式覆盖成 0：老构建目录的 CMakeCache 里还留着 =1
    avif.define("CMAKE_DISABLE_FIND_PACKAGE_libyuv", "0");

    eprintln!("building libavif");

    let local_pc_files = env::join_paths(pc_paths).unwrap();

    eprintln!("pc=\"{:?}\"", local_pc_files);
    avif.env("PKG_CONFIG_PATH", local_pc_files);
    avif.define("CMAKE_DISABLE_FIND_PACKAGE_libsharpyuv", "1");

    avif.profile(if env::var("PROFILE").expect("PROFILE") == "release" {
        "Release"
    } else {
        "Debug"
    })
    .configure_arg("-DCMAKE_INSTALL_LIBDIR=lib");
    if env::var("LIBAVIF_CROSS_WIN32").is_ok() {
        avif.configure_arg("-T host=x64").configure_arg("-A Win32");
    }

    let avif_built = avif.build();

    println!(
        "cargo:rustc-link-search=native={}",
        avif_built.join("lib").display()
    );
    println!("cargo:rustc-link-lib=static=avif");
    // 本地 vendor 时新增：把静态库绝对路径暴露给上层 crate（配合 links = "avif"）
    println!(
        "cargo:staticlib={}",
        avif_built.join("lib").join("libavif.a").display()
    );
    // libavif.a 里 reformat_libyuv.c 引用了 yuv 的符号，最终 .so 的链接组里也要带上它
    println!("cargo:yuv_staticlib={}", yuv_staticlib.display());

    println!("cargo:outdir={}", out_dir.display());
}
