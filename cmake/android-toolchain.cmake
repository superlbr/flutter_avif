# 包装 NDK 官方 toolchain。
#
# 为什么需要这一层：libavif-sys 通过 cmake crate 调 CMake，只允许我们用环境变量传
# CMAKE_TOOLCHAIN_FILE，没法传 -DANDROID_ABI / -DANDROID_PLATFORM。直接指向 NDK 的
# 原始 toolchain 时，这两个变量缺失会让 CMake 退回宿主（macOS）平台判断，进而给
# clang 追加 -arch arm64/-isysroot，报 "unsupported option '-arch'"。
#
# 所以在这里先从环境变量补齐，再 include 官方文件。

if(DEFINED ENV{ANDROID_ABI})
  set(ANDROID_ABI "$ENV{ANDROID_ABI}")
endif()
if(DEFINED ENV{ANDROID_PLATFORM})
  set(ANDROID_PLATFORM "$ENV{ANDROID_PLATFORM}")
endif()
if(DEFINED ENV{ANDROID_NDK_HOME})
  set(ANDROID_NDK "$ENV{ANDROID_NDK_HOME}")
endif()

if(NOT DEFINED ANDROID_ABI)
  message(FATAL_ERROR "ANDROID_ABI 未设置（arm64-v8a / armeabi-v7a）")
endif()

include("$ENV{ANDROID_NDK_HOME}/build/cmake/android.toolchain.cmake")
