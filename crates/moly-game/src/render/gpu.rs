//! GPU object lifetime helpers.
//!
//! On the WebGPU backend a dropped buffer, texture, sampler or bind group is
//! only released when the browser garbage-collects its JS wrapper, so objects
//! that are identical from frame to frame are created once and reused here.

/// Stores `value` into `slot` only when a bit differs and reports whether it
/// wrote. Floats compare by bit pattern, so a store is skipped only when it
/// would write the very same bits (`-0.0` still replaces `0.0`).
pub(crate) fn store_bits<const N: usize>(slot: &mut [f32; N], value: [f32; N]) -> bool {
    if slot.map(f32::to_bits) == value.map(f32::to_bits) {
        return false;
    }
    *slot = value;
    true
}
