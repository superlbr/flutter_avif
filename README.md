# flutter_avif for Android — prebuilt `libflutter_avif.so`

[![CI](https://github.com/superlbr/flutter_avif/actions/workflows/build-android.yml/badge.svg)](https://github.com/superlbr/flutter_avif/actions/workflows/build-android.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)

A self-contained build of the [`flutter_avif`](https://github.com/yekeskin/flutter_avif) plugin
together with a precompiled `libflutter_avif.so` for Android, tuned for decoding **8-bit 4:2:0**
AVIF images.

The repository vendors every native dependency it needs (libavif, dav1d, libyuv), so a clone plus
one command produces both ABIs — no forks, no submodules, no network fetch at build time.

## Contents

```
flutter_avif/                     Dart-facing API
flutter_avif_platform_interface/  FFI bindings + protobuf models
flutter_avif_android/             Android implementation; prebuilt .so in jniLibs/
flutter_avif_ios/                 iOS implementation (unchanged upstream sources)
rust/                             cdylib crate -> libflutter_avif.so
vendors/libavif-sys/              libavif 1.0.4 sources + CMake build script
vendors/libdav1d-sys/             dav1d 1.4.3 sources + meson build script
vendors/libyuv/                   libyuv 1974 sources (no JPEG, no tests)
bench/                            host-side reformat micro-benchmark (not part of the Android build)
cmake/android-toolchain.cmake     NDK toolchain wrapper (see Design notes)
build.sh                          one command, one or both ABIs
.github/workflows/build-android.yml
```

Paths are wired with `[patch.crates-io]` in `rust/Cargo.toml`; nothing is fetched from crates.io.

## Changes from upstream

- **libyuv SIMD fast path enabled.** Upstream's `libavif-sys` build disables libyuv, so
  `avifImageYUVToRGB` runs libavif's scalar C path. This build vendors libyuv (version **1974**,
  above libavif's 1813 threshold for the `I*ToARGBMatrixFilter` variants) and compiles
  `src/reformat_libyuv.c` instead.
- **No full-frame RGBA copies in Rust.** The frame buffer is allocated once and handed to
  `avifImageYUVToRGB` directly, instead of `avifRGBImageAllocatePixels()` + `to_vec()` +
  `FreePixels()`. For a 1080×2346 page that removes one 10.13 MB copy and keeps two 10.13 MB
  buffers from being alive at the same time.
- **dav1d 1.4.3** (the newest dav1d with vendored sources on crates.io).
- **rav1e removed**, with `LTO`, `codegen-units = 1` and `--strip-unneeded` enabled; decoding-only
  binaries do not need an encoder.
- **Optional `max-threads` feature**, off by default. AVIF frames are usually a single tile, and a
  single tile gives no measurable parallel speedup while costing extra YUV buffers, so threading
  is opt-in rather than baked in.

## Size

Both ABIs, same toolchain, same flags:

| ABI | Previous build (raw) | This build (raw) | **This build (stripped)** | Previous (stripped) | Delta (stripped) |
|---|---|---|---|---|---|
| arm64-v8a | 4,362,296 | 1,921,936 | **1,526,112** | 3,309,496 | −1,783,384 (−53.9%) |
| armeabi-v7a | 3,061,960 | 1,616,472 | **1,245,132** | 2,405,044 | −1,159,912 (−48.2%) |

The libyuv fast path is what makes this build *larger* than a libyuv-less one: +156,288 B
(+11.4%) on arm64-v8a and +102,176 B (+8.9%) on armeabi-v7a, stripped. That cost buys the
speedup below.

Only six `#[no_mangle]` Dart FFI entry points are exported; everything else (`avif*`, `dav1d_*`,
libyuv internals) stays local and is removable by `--gc-sections` + LTO.

## Benchmark

Micro-benchmark of `avifImageYUVToRGB` alone — same decoded YUV planes, same output buffer,
50 iterations, scalar and libyuv builds of libavif side by side.

Environment: **host, Apple Silicon (arm64 macOS)**, `clang -O2`, `n = 50`, median reported.
These are desktop numbers, not device numbers.

| Input | Size | Scalar C | libyuv | Speedup |
|---|---|---|---|---|
| detailed page | 1080×2346 | 20.42 ms | **0.78 ms** | 26.2× |
| mid-detail page | 1080×2346 | 19.70 ms | **0.79 ms** | 24.9× |
| flat page | 1080×2346 | 19.23 ms | **0.79 ms** | 24.3× |
| reader width (synthetic YUV420) | 585×1271 | 5.56 ms | **0.25 ms** | 22.2× |
| reader width (synthetic YUV420) | 585×2346 | 11.53 ms | **0.48 ms** | 24.0× |

Reproduce with `bench/run_bench.sh <file.avif|synth:WxH> [...]`; it builds both libavif variants
against the host toolchain and prints per-iteration medians.

Caveats, stated plainly:

- The "2–3×" figure often quoted for libyuv is not published by libavif and does not appear in its
  sources or CMake comments; the numbers above are what this repository measured.
- **On-device numbers are not collected yet.** Scaling by the scalar-path host-to-device ratio
  (~1.15×) puts reformat at roughly 0.9 ms on a phone.
- **The ~26 ms end-to-end figure for a 1080×2346 page is an estimate**, derived from
  47.7 ms end-to-end minus 22.5 ms of scalar reformat plus ~0.9 ms of libyuv reformat. It has not
  been measured on a device.

Because reformat drops to sub-millisecond, row-stripe parallelism was evaluated and rejected:
thread wake-up plus barrier costs 0.1–0.3 ms by itself, and N in-flight stripe buffers raise peak
memory. `max-threads` therefore stays off by default.

## Pixel difference

libyuv uses fixed-point rounding and its own limited/full-range handling, so its output is not
bit-identical to libavif's scalar path. Byte-wise comparison of the RGBA output (10,134,720 B per
1080×2346 frame):

| Input | Max delta | Bytes differing by 1 | Bytes differing by >1 |
|---|---|---|---|
| detailed page | **2** (B channel only, 522 B) | 25.64% | 0.005% |
| mid-detail page | **1** | 0.01% | 0 |
| flat page | **0** | 0% | 0 |

A delta of 1/255 is 0.4% of the channel range; the handful of 2/255 outliers is 0.8%, in one
channel, at the rounding boundary. The alpha channel is byte-identical. The difference is a
rounding artifact and is not visible.

## Build

```bash
./build.sh                  # both ABIs
./build.sh arm64-v8a        # one ABI
./build.sh --features max-threads   # opt-in decoder threading
```

Output is written to `dist/<abi>/libflutter_avif.so` and mirrored into
`flutter_avif_android/android/src/main/jniLibs/<abi>/`.

| Requirement | Version used | Notes |
|---|---|---|
| Rust | ≥ 1.75 (1.96.0 verified) | `rustup target add aarch64-linux-android armv7-linux-androideabi` |
| cargo-ndk | 4.1.2 | `cargo install cargo-ndk --version 4.1.2 --locked` |
| meson | ≥ 1.3 | dav1d is built with meson |
| ninja | any recent | meson backend |
| Android NDK | 28.2.13676358 (r28c) | located via `ANDROID_NDK_HOME` or the SDK directory |

`build.sh` finishes with an `llvm-nm -D -u` check and fails the build if any `dav1d_*` / `avif*`
symbol is left undefined.

## Diagnostics

Per-stage timing is built in and off by default. Set the `FLUTTER_AVIF_TRACE` environment
variable in the process (Android: `adb shell setprop wrap.<applicationId> "env FLUTTER_AVIF_TRACE=1"`,
debuggable builds only; reset with an empty value) and every decoded frame logs one fixed-format
line under tag `avif-trace`:

```
[avif-trace] frame 896x1200 t1=412 t2=7898 t3=595 t4=3510 t5=-1 total=12415
```

Values are microseconds and `t1`…`t4` are contiguous deltas — decoder creation + `Parse`,
`avifDecoderNextImage`, `avifImageYUVToRGB`, protobuf packing — with `total` their sum. `t5` is
always `-1`: `postCObject` happens inside the `allo_isolate` task wrapper, so its return is not
observable from Rust; derive it on the Dart side as *(message arrival − total)*.

## CI and artifacts

Workflow: **`build-android-so`** ([build-android.yml](.github/workflows/build-android.yml)).

- Every push and pull request builds a matrix of `arm64-v8a` and `armeabi-v7a` on `ubuntu-latest`.
- Each job uploads one artifact: **`libflutter_avif-arm64-v8a`** and
  **`libflutter_avif-armeabi-v7a`** (the stripped `.so`).
- Pushing a `v*` tag additionally attaches both binaries to a GitHub Release.

## Design notes

Three problems in this toolchain fail *silently* — the build succeeds and the output is still
unusable — so they are handled explicitly rather than left to defaults.

1. **CMake must receive a `CMAKE_TOOLCHAIN_FILE`.** `libavif-sys` builds through the `cmake`
   crate, which forwards only the `CMAKE_TOOLCHAIN_FILE` environment variable and cannot pass
   `-DANDROID_ABI` / `-DANDROID_PLATFORM`. Pointing it straight at the NDK's official toolchain
   file makes CMake fall back to host mode and append `-arch arm64 -isysroot`. `cmake/android-toolchain.cmake`
   fills those variables from the environment first, then includes the official file.

2. **dav1d needs an Android meson cross file.** `libdav1d-sys` ships cross files for Windows,
   Linux and wasm only. Without one, meson compiles dav1d with the host compiler into a Mach-O
   `libdav1d.a` whose symbols are `_dav1d_open`; lld skips that archive when linking the ELF, and
   because shared objects allow undefined symbols the link still succeeds — producing an 862 KB
   `.so` with every `dav1d_*` undefined that fails at `dlopen`. `vendors/libdav1d-sys/build.rs`
   generates a cross file from the NDK's `<arch><api>-clang` wrappers instead.

3. **LTO drops the static-library link directives.** With `lto = true`, the fat-LTO link path no
   longer honours `cargo:rustc-link-lib=static=avif|dav1d` from dependency build scripts. The
   archives are therefore passed explicitly: `rust/build.rs` reads `DEP_AVIF_STATICLIB`,
   `DEP_DAV1D_STATICLIB` and `DEP_AVIF_YUV_STATICLIB` (exposed by `links` + `cargo:staticlib=` /
   `cargo:yuv_staticlib=` in the `-sys` crates) and emits them as `cargo:rustc-link-arg` inside
   `--start-group` / `--end-group`, since libavif, dav1d and libyuv reference each other.

## License

The code in this repository — `build.sh`, the CI workflow, the benchmark sources, the CMake
toolchain wrapper and the glue in `rust/` — is released under the **MIT License**; see
[LICENSE](LICENSE), Copyright (c) 2026 superlbr.

The MIT license does **not** extend to the vendored dependencies. Each keeps its upstream license,
and each license file is kept in its original location verbatim.

### Third-party components

| Component | Version | License | License file |
|---|---|---|---|
| [libavif](https://github.com/AOMediaCodec/libavif) | 1.0.4 | BSD-2-Clause | `vendors/libavif-sys/libavif/LICENSE` |
| [dav1d](https://code.videolan.org/videolan/dav1d) | 1.4.3 | BSD-2-Clause | `vendors/libdav1d-sys/vendor/COPYING` |
| [libyuv](https://chromium.googlesource.com/libyuv/libyuv) | 1974 | BSD-3-Clause | `vendors/libyuv/LICENSE` (plus `PATENTS`) |
| [flutter_avif](https://github.com/yekeskin/flutter_avif) | 3.1.0 | MIT | `flutter_avif*/LICENSE` |

If you redistribute the prebuilt `.so`, the BSD-2-Clause and BSD-3-Clause notices above apply to
the corresponding code inside it; the MIT notice covers this repository's own code only.
