//! The two calls into native code that converting a model needs: mapping the weights file, and ggml's quantizer.
//!
//! This is the crate's only module that opts out of `deny(unsafe_code)`. Everything else in the converter is safe Rust over the
//! slices and vectors these functions return.
#![allow(
    unsafe_code,
    reason = "a memory map and the quantizer of the ggml that whisper.cpp links are foreign calls"
)]

use alloc::vec;
use alloc::vec::Vec;
use std::fs::File;
use std::io;
use std::path::Path;

use memmap2::Mmap;
use whisper_rs_sys as sys;

use crate::conversion::Precision;

/// The ggml version of quantized data the linked ggml writes, which a file records beside its `ftype`.
pub(crate) const QUANTIZATION_VERSION: i32 = sys::GGML_QNT_VERSION as i32;

/// The multiplier that puts that version into the `ftype` field.
pub(crate) const QUANTIZATION_VERSION_FACTOR: i32 = sys::GGML_QNT_VERSION_FACTOR as i32;

/// Maps `path` read-only. The weights of a large model are read from the map as they are written out, so they are never held whole.
///
/// The caller must not change or shorten the file while the map is alive; the folder a download wrote is not touched meanwhile.
pub(crate) fn map(path: &Path) -> io::Result<Mmap> {
    let file = File::open(path)?;
    // SAFETY: the map is read-only and the file is not written while it is held (see above).
    unsafe { Mmap::map(&file) }
}

#[allow(
    clippy::unnecessary_cast,
    reason = "bindgen types the enum constants u32 on Linux and i32 on Windows"
)]
/// The `ggml_type` a tensor of `precision` carries in the file.
pub(crate) fn type_id(precision: Precision) -> i32 {
    (match precision {
        Precision::F16 => sys::ggml_type_GGML_TYPE_F16,
        Precision::Q5_0 => sys::ggml_type_GGML_TYPE_Q5_0,
        Precision::Q5_1 => sys::ggml_type_GGML_TYPE_Q5_1,
        Precision::Q8_0 => sys::ggml_type_GGML_TYPE_Q8_0,
    }) as i32
}

#[allow(
    clippy::unnecessary_cast,
    reason = "bindgen types the enum constants u32 on Linux and i32 on Windows"
)]
/// The `ggml_ftype` the file's header holds for `precision`, before the quantization version is added.
pub(crate) fn ftype_id(precision: Precision) -> i32 {
    (match precision {
        Precision::F16 => sys::ggml_ftype_GGML_FTYPE_MOSTLY_F16,
        Precision::Q5_0 => sys::ggml_ftype_GGML_FTYPE_MOSTLY_Q5_0,
        Precision::Q5_1 => sys::ggml_ftype_GGML_FTYPE_MOSTLY_Q5_1,
        Precision::Q8_0 => sys::ggml_ftype_GGML_FTYPE_MOSTLY_Q8_0,
    }) as i32
}

/// The values of rows of `row_len` floats, quantized to `precision`, or `None` when the row does not divide into the type's
/// blocks (32 values) or the quantizer wrote a different size than ggml says the rows take.
pub(crate) fn quantize(precision: Precision, values: &[f32], row_len: usize) -> Option<Vec<u8>> {
    if precision == Precision::F16
        || row_len == 0
        || !row_len.is_multiple_of(32)
        || !values.len().is_multiple_of(row_len)
    {
        return None;
    }
    let ty = type_id(precision) as sys::ggml_type;
    let rows = values.len() / row_len;
    // SAFETY: `ggml_row_size` only reads the type's table.
    let row_bytes = unsafe { sys::ggml_row_size(ty, row_len as i64) };
    let mut out = vec![0u8; rows * row_bytes];
    // SAFETY: `values` holds `rows * row_len` floats and `out` holds `rows` rows of the type's size, which are the extents the
    // call reads and writes; it keeps no pointer. A null importance matrix is allowed for these types.
    let written = unsafe {
        sys::ggml_quantize_chunk(
            ty,
            values.as_ptr(),
            out.as_mut_ptr().cast(),
            0,
            rows as i64,
            row_len as i64,
            core::ptr::null(),
        )
    };
    (written == out.len()).then_some(out)
}
