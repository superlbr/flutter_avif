#!/usr/bin/env bash
# 构建 libflutter_avif.so（arm64-v8a + armeabi-v7a），strip 后同时写到：
#   dist/<abi>/libflutter_avif.so
#   flutter_avif_android/android/src/main/jniLibs/<abi>/libflutter_avif.so
#
# 依赖：
#   rustc/cargo  >= 1.75（本仓库验证于 1.96.0），已装 aarch64-linux-android / armv7-linux-androideabi
#   cargo-ndk    4.x（验证于 4.1.2）
#   meson + ninja（libdav1d-sys 用 meson 编译内置的 dav1d 1.4.3）
#   Android NDK  >= r26（验证于 r28c / 28.2.13676358）
#
# 用法：
#   ./build.sh                        # 两个 ABI
#   ./build.sh arm64-v8a              # 只编一个
#   ./build.sh --features max-threads # 额外 cargo 参数（打开解码多线程）
set -euo pipefail

ROOT="$(cd "$(dirname "$0")" && pwd)"
DIST="$ROOT/dist"

# ---------- 1. 环境检查 ----------
need() { command -v "$1" >/dev/null 2>&1 || { echo "缺少 $1：$2" >&2; exit 1; }; }
need cargo "curl https://sh.rustup.rs -sSf | sh"
need meson "pip3 install --user meson ninja（或 brew install meson ninja）"
need ninja "pip3 install --user meson ninja（或 brew install meson ninja）"
cargo ndk --version >/dev/null 2>&1 || { echo "缺少 cargo-ndk：cargo install cargo-ndk" >&2; exit 1; }

# NDK 路径：优先 $ANDROID_NDK_HOME，其次 $ANDROID_SDK_ROOT/ndk，最后默认 sdk 目录
NDK="${ANDROID_NDK_HOME:-}"
if [ -z "$NDK" ]; then
  SDK="${ANDROID_SDK_ROOT:-${ANDROID_HOME:-$HOME/Library/Android/sdk}}"
  NDK="$(ls -d "$SDK"/ndk/* 2>/dev/null | sort -V | tail -1 || true)"
fi
[ -n "$NDK" ] && [ -d "$NDK" ] || { echo "找不到 NDK，请设置 ANDROID_NDK_HOME" >&2; exit 1; }
export ANDROID_NDK_HOME="$NDK"
echo "NDK: $NDK"

# 这两个 build script 都要用：
#   libavif-sys  → cmake crate 只认 CMAKE_TOOLCHAIN_FILE 环境变量，不能传 -DANDROID_ABI，
#                  所以指向本仓库的包装文件（先补 ANDROID_ABI/ANDROID_PLATFORM 再 include 官方）
#   libdav1d-sys → meson 的 android cross file 里要拼 <arch><api>-clang
export CMAKE_TOOLCHAIN_FILE="$ROOT/cmake/android-toolchain.cmake"
export ANDROID_PLATFORM="android-21"

HOST_TAG="$(uname -s | tr 'A-Z' 'a-z')-x86_64"
BIN="$NDK/toolchains/llvm/prebuilt/$HOST_TAG/bin"
STRIP="$BIN/llvm-strip"
NM="$BIN/llvm-nm"

# ---------- 2. 解析参数 ----------
ABIS=()
CARGO_ARGS=()
for arg in "$@"; do
  case "$arg" in
    arm64-v8a|armeabi-v7a|x86_64) ABIS+=("$arg") ;;
    *) CARGO_ARGS+=("$arg") ;;
  esac
done
[ ${#ABIS[@]} -gt 0 ] || ABIS=(arm64-v8a armeabi-v7a)

triple_of() {
  case "$1" in
    arm64-v8a)   echo aarch64-linux-android ;;
    armeabi-v7a) echo armv7-linux-androideabi ;;
    x86_64)      echo x86_64-linux-android ;;
  esac
}

# ---------- 3. 编译 ----------
cd "$ROOT/rust"

echo "构建：${ABIS[*]} ${CARGO_ARGS[*]:-}"
# 每个 ABI 单独 build：ANDROID_ABI 是环境变量，cmake 和 meson 的 cross file 都从它取，
# 一次传多个 -t 时没法让两边各拿各的值。
for abi in "${ABIS[@]}"; do
  export ANDROID_ABI="$abi"
  # macOS 自带 bash 3.2：空数组在 set -u 下展开会报 unbound，要加这层保护
  cargo ndk -t "$abi" --platform 21 build --release ${CARGO_ARGS[@]+"${CARGO_ARGS[@]}"}
done

# ---------- 4. 产出 + strip + 自检 ----------
mkdir -p "$DIST"
printf "\n%-14s %12s %12s %8s\n" "ABI" "raw" "stripped" "-"
for abi in "${ABIS[@]}"; do
  TRIPLE="$(triple_of "$abi")"
  SRC="$ROOT/rust/target/$TRIPLE/release/libflutter_avif.so"
  [ -f "$SRC" ] || { echo "没找到 $SRC" >&2; exit 1; }

  # 自检：dav1d/avif 的符号必须已经静态链进来。共享库允许未定义符号，
  # 所以「链接成功」不等于「能用」，这里必须显式查一遍。
  if UND=$("$NM" -D -u "$SRC" | grep -E ' (dav1d_|avif)'); then
    echo "链接不完整，以下符号仍未定义：" >&2
    echo "$UND" >&2
    exit 1
  fi

  RAW=$(stat -f%z "$SRC" 2>/dev/null || stat -c%s "$SRC")
  mkdir -p "$DIST/$abi"
  cp "$SRC" "$DIST/$abi/libflutter_avif.so"
  "$STRIP" --strip-unneeded "$DIST/$abi/libflutter_avif.so"

  # 直接放进插件包，clone 下来就能当依赖用
  JNI="$ROOT/flutter_avif_android/android/src/main/jniLibs/$abi"
  mkdir -p "$JNI"
  cp "$DIST/$abi/libflutter_avif.so" "$JNI/libflutter_avif.so"

  DST=$(stat -f%z "$DIST/$abi/libflutter_avif.so" 2>/dev/null || stat -c%s "$DIST/$abi/libflutter_avif.so")
  printf "%-14s %12s %12s %7.1f%%\n" "$abi" "$RAW" "$DST" "$(awk "BEGIN{printf (1-$DST/$RAW)*100}")"
done

echo
echo "产物：$DIST/{$(IFS=,; echo "${ABIS[*]}")}/libflutter_avif.so"
echo "已同步到：flutter_avif_android/android/src/main/jniLibs/<abi>/"
