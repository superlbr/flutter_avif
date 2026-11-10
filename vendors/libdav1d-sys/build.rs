use std::env;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    if env::var_os("DOCS_RS").is_some() {
        return;
    }

    let out_dir = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let install_dir = out_dir.join("install");
    let mut meson = Command::new("meson");

    meson.env("DESTDIR", &install_dir);

    let target = env::var("TARGET").expect("TARGET");

    if target == "i686-pc-windows-msvc" {
        meson.arg("--cross-file").arg("i686-win-msvc.meson");
    }
    if target == "x86_64-pc-windows-gnu" {
        meson.arg("--cross-file").arg("x86_64-w64-mingw32.meson");
    }
    if target == "aarch64-unknown-linux-gnu" {
        meson
            .arg("--cross-file")
            .arg("aarch64-unknown-linux-gnu.meson");
    }
    if target == "wasm32-unknown-emscripten" {
        meson
            .arg("--cross-file")
            .arg("wasm32-unknown-emscripten.meson");
    }
    // 本地 vendor 时的改动：上游没有 Android 的 cross file，缺失时 meson 会退回宿主
    // 工具链，编出 Mach-O 格式的 libdav1d.a（符号是 `_dav1d_open`）。这个 .a 链接进
    // ELF 的 .so 时会被 lld 静默跳过（共享库允许未定义符号，所以不报错），最终产物里
    // dav1d_* 全是 UND，运行时 dlopen 失败。这里按 NDK 的 clang 包装脚本现生成一份。
    if target.contains("android") {
        meson
            .arg("--cross-file")
            .arg(android_cross_file(&out_dir, &target));
    }

    let s = meson
        .arg("--default-library=static")
        .arg("--buildtype")
        .arg(if env::var("PROFILE").expect("PROFILE") == "release" {
            "release"
        } else {
            "debug"
        })
        .arg("--prefix=/")
        .arg("--libdir=lib")
        .arg(&out_dir)
        .arg("vendor")
        .status()
        .expect("This crate requires meson (and ninja) to be installed: https://mesonbuild.com/");
    assert!(s.success(), "meson failed");

    eprintln!("Installing dav1d into {:?}", install_dir);
    let s = Command::new("ninja")
        .env("DESTDIR", &install_dir)
        .current_dir(&out_dir)
        .arg("install")
        .status()
        .expect("This crate requires ninja to be installed: https://ninja-build.org/");
    assert!(s.success(), "ninja failed");

    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap();
    let target_env = env::var("CARGO_CFG_TARGET_ENV").unwrap();

    let mut unix_path = install_dir.join("lib").join("libdav1d.a");
    if !unix_path.exists() {
        unix_path = std::fs::read_dir(install_dir.join("lib"))
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| !e.file_name().to_str().map_or(false, |n| n.starts_with('.')))
            .map(|e| e.path().join("libdav1d.a"))
            .find(|p| p.exists())
            .expect("can't find libdav1d.a in install dir");
    }
    let staticlib_path = if target_os == "windows" && target_env == "msvc" {
        let win_path = install_dir.join("lib").join("dav1d.lib");
        if !win_path.exists() {
            std::fs::rename(&unix_path, &win_path).unwrap();
        }
        win_path
    } else {
        assert!(unix_path.exists(), "ninja didn't install the library");
        unix_path
    };
    println!(
        "cargo:rustc-link-search=native={}",
        staticlib_path.parent().unwrap().display()
    );
    println!("cargo:rustc-link-lib=static=dav1d");

    // DEP_DAV1D_PRODUCTS variable to set as cmake include path
    println!("cargo:products={}", install_dir.display());
    // DEP_DAV1D_PKGCONFIG variable to add to pkg-config's search path
    println!(
        "cargo:pkgconfig={}",
        install_dir.join("lib").join("pkgconfig").display()
    );
    // DEP_DAV1D_STATICLIB with actual path to the library
    println!("cargo:staticlib={}", staticlib_path.display());
    // DEP_DAV1D_INCLUDE with header include path
    println!("cargo:include={}", install_dir.join("include").display());

    #[cfg(feature = "generate")]
    generate_bindings();
}

/// 按 NDK 里的 clang 包装脚本生成 meson cross file，供 Android 目标交叉编译 dav1d。
fn android_cross_file(out_dir: &std::path::Path, target: &str) -> PathBuf {
    let ndk = env::var("ANDROID_NDK_HOME")
        .or_else(|_| env::var("ANDROID_NDK_ROOT"))
        .expect("交叉编译 dav1d 需要 ANDROID_NDK_HOME");

    // NDK 的 clang 包装脚本命名：<arch><api>-clang。
    // 注意 armv7 的 rust triple 是 armv7-linux-androideabi，而 NDK 里叫 armv7a-linux-androideabi。
    let (clang_arch, cpu_family, cpu) = match target {
        "aarch64-linux-android" => ("aarch64-linux-android", "aarch64", "aarch64"),
        "armv7-linux-androideabi" => ("armv7a-linux-androideabi", "arm", "armv7a"),
        "i686-linux-android" => ("i686-linux-android", "x86", "i686"),
        "x86_64-linux-android" => ("x86_64-linux-android", "x86_64", "x86_64"),
        other => panic!("未支持的 Android 目标: {other}"),
    };

    // ANDROID_PLATFORM 形如 android-21
    let api = env::var("ANDROID_PLATFORM")
        .ok()
        .and_then(|p| p.rsplit('-').next().map(str::to_owned))
        .unwrap_or_else(|| "21".to_string());

    let prebuilt = std::fs::read_dir(std::path::Path::new(&ndk).join("toolchains/llvm/prebuilt"))
        .expect("NDK 里找不到 toolchains/llvm/prebuilt")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .find(|p| p.join("bin").is_dir())
        .expect("NDK 的 llvm prebuilt 目录为空");
    let bin = prebuilt.join("bin");

    let clang = |name: String| {
        let p = bin.join(name);
        assert!(p.exists(), "NDK 里找不到编译器: {}", p.display());
        p.display().to_string()
    };
    let cc = clang(format!("{clang_arch}{api}-clang"));
    let cpp = clang(format!("{clang_arch}{api}-clang++"));
    let ar = clang("llvm-ar".to_string());
    let ranlib = clang("llvm-ranlib".to_string());
    let strip = clang("llvm-strip".to_string());

    let mut s = String::new();
    s.push_str("[binaries]\n");
    s.push_str(&format!("c = '{cc}'\n"));
    s.push_str(&format!("cpp = '{cpp}'\n"));
    s.push_str(&format!("ar = '{ar}'\n"));
    s.push_str(&format!("ranlib = '{ranlib}'\n"));
    s.push_str(&format!("strip = '{strip}'\n"));
    if cpu_family == "arm" {
        // NDK 的 armv7 默认 -mfpu=vfpv3-d16，dav1d 的 arm 汇编需要 NEON
        s.push_str("\n[built-in options]\n");
        s.push_str("c_args = ['-march=armv7-a', '-mfpu=neon', '-mfloat-abi=softfp']\n");
        s.push_str("cpp_args = ['-march=armv7-a', '-mfpu=neon', '-mfloat-abi=softfp']\n");
    }
    s.push_str("\n[host_machine]\n");
    s.push_str("system = 'android'\n");
    s.push_str(&format!("cpu_family = '{cpu_family}'\n"));
    s.push_str(&format!("cpu = '{cpu}'\n"));
    s.push_str("endian = 'little'\n");

    let path = out_dir.join("android-cross.meson");
    std::fs::write(&path, s).expect("写 android-cross.meson 失败");
    path
}

#[cfg(feature = "generate")]
fn generate_bindings() {
    let out_path = PathBuf::from(env::var_os("OUT_DIR").unwrap());

    let bindings = bindgen::Builder::default()
        .header("vendor/include/dav1d/dav1d.h")
        .parse_callbacks(Box::new(bindgen::CargoCallbacks::new()))
        .constified_enum_module("Dav1dInloopFilterType")
        .default_enum_style(bindgen::EnumVariation::Rust {
            non_exhaustive: true,
        })
        .layout_tests(false)
        .allowlist_item("^[Dd][aA][vV].*")
        .blocklist_item("^_.*")
        .clang_args(&[
            "-I",
            "vendor/include/dav1d/",
            "-I",
            out_path.join("install/include/dav1d/").to_str().unwrap(),
        ])
        .generate()
        .expect("Unable to generate bindings");

    // Write the bindings to the $OUT_DIR/bindings.rs file.
    bindings
        .write_to_file(out_path.join("bindings.rs"))
        .expect("Couldn't write bindings!");
}
