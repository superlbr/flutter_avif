#[cfg(feature = "codec-aom")]
extern crate libaom_sys; // mark it as used

#[cfg(feature = "codec-dav1d")]
extern crate libdav1d_sys; // mark it as used

// 本地 vendor 时删除：rav1e 编码器整个移除（本仓库只做解码），详见 README「改动 3」。

#[allow(non_camel_case_types)]
#[allow(non_snake_case)]
mod ffi;
pub use ffi::*;
