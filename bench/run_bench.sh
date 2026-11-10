#!/usr/bin/env bash
# 在宿主上对拍 avifImageYUVToRGB 的两条路径：libavif 标量 C / libyuv SIMD。
# 只用于出数字，不参与 Android 构建。
#
#   ./bench/run_bench.sh /path/a.avif /path/b.avif ...
#   ./bench/run_bench.sh synth:585x1271          # 没有样本时用合成 YUV420
#
# 依赖：cmake、ninja、meson（dav1d 用）、一个 C 编译器。
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
V="$ROOT/vendors"
OUT="${BENCH_OUT:-$ROOT/bench/.build}"
ITERS="${ITERS:-50}"

[ $# -ge 1 ] || { echo "用法: $0 <in.avif|synth:WxH> [...]" >&2; exit 2; }

mkdir -p "$OUT"

# 1) dav1d（宿主）——bench 要真解码一张 AVIF
if [ ! -f "$OUT/dav1d/lib/libdav1d.a" ]; then
  echo "== dav1d =="
  meson setup "$OUT/dav1d-build" "$V/libdav1d-sys/vendor" --buildtype=release \
    --default-library=static -Denable_tools=false -Denable_examples=false -Denable_tests=false \
    --prefix "$OUT/dav1d" >/dev/null
  ninja -C "$OUT/dav1d-build" >/dev/null
  mkdir -p "$OUT/dav1d/lib" "$OUT/dav1d/include/dav1d"
  cp "$OUT/dav1d-build/src/libdav1d.a" "$OUT/dav1d/lib/libdav1d.a"
  cp "$V/libdav1d-sys/vendor/include/dav1d"/*.h "$OUT/dav1d/include/dav1d/"
  cp "$OUT/dav1d-build/include/dav1d/version.h" "$OUT/dav1d/include/dav1d/"
fi

# 2) libyuv（宿主，静态 + PIC）
if [ ! -f "$OUT/yuv/lib/libyuv.a" ]; then
  echo "== libyuv =="
  cmake -S "$V/libyuv" -B "$OUT/yuv-build" -G Ninja -DCMAKE_BUILD_TYPE=Release \
    -DBUILD_SHARED_LIBS=OFF -DLIBYUV_DISABLE_JPEG=ON -DUNIT_TEST=OFF \
    -DCMAKE_POSITION_INDEPENDENT_CODE=ON -DCMAKE_INSTALL_PREFIX="$OUT/yuv" >/dev/null
  cmake --build "$OUT/yuv-build" --target install >/dev/null
fi

# 3) libavif 两份：关 libyuv / 开 libyuv
build_avif() { # $1=目录名 $2...=额外 -D
  local name="$1"; shift
  [ -f "$OUT/$name/lib/libavif.a" ] && return 0
  echo "== libavif ($name) =="
  cmake -S "$V/libavif-sys/libavif" -B "$OUT/$name-build" -G Ninja \
    -DCMAKE_BUILD_TYPE=Release -DBUILD_SHARED_LIBS=OFF -DAVIF_ENABLE_WERROR=0 \
    -DAVIF_BUILD_APPS=OFF -DAVIF_BUILD_TESTS=OFF \
    -DAVIF_CODEC_DAV1D=ON -DAVIF_CODEC_AOM=OFF -DAVIF_CODEC_RAV1E=OFF \
    -DAVIF_CODEC_SVT=OFF -DAVIF_CODEC_LIBGAV1=OFF \
    -DDAV1D_INCLUDE_DIR="$OUT/dav1d/include" -DDAV1D_LIBRARY="$OUT/dav1d/lib/libdav1d.a" \
    -DCMAKE_DISABLE_FIND_PACKAGE_libsharpyuv=1 \
    -DCMAKE_INSTALL_PREFIX="$OUT/$name" "$@" >/dev/null
  cmake --build "$OUT/$name-build" --target install >/dev/null
}
build_avif avif-scalar -DCMAKE_DISABLE_FIND_PACKAGE_libyuv=1
build_avif avif-yuv -DLIBYUV_INCLUDE_DIR="$OUT/yuv/include" -DLIBYUV_LIBRARY="$OUT/yuv/lib/libyuv.a"

# 4) 编 bench（C++ 运行时是给 libyuv 用的）
CXXLIB=""
case "$(uname -s)" in Darwin) CXXLIB="-lc++" ;; *) CXXLIB="-lstdc++" ;; esac
cc -O2 -o "$OUT/bench_scalar" "$ROOT/bench/bench_reformat.c" -I "$OUT/avif-scalar/include" \
  "$OUT/avif-scalar/lib/libavif.a" "$OUT/dav1d/lib/libdav1d.a" -lm
cc -O2 -o "$OUT/bench_yuv" "$ROOT/bench/bench_reformat.c" -I "$OUT/avif-yuv/include" \
  "$OUT/avif-yuv/lib/libavif.a" "$OUT/yuv/lib/libyuv.a" "$OUT/dav1d/lib/libdav1d.a" -lm $CXXLIB

# 5) 跑
for f in "$@"; do
  echo
  echo "########## $f ##########"
  "$OUT/bench_scalar" "$f" "" "$ITERS" 2>/dev/null | grep -E "^(file|decode|reformat)"
  echo "--- libyuv ---"
  "$OUT/bench_yuv" "$f" "" "$ITERS" 2>/dev/null | grep -E "^(decode|reformat)"
done
