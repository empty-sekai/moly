//! The browser build's global allocator: dlmalloc over a system layer that
//! grows the WebAssembly memory geometrically.
//!
//! dlmalloc asks its system layer for what one allocation needs, rounded up to
//! 64 KiB, and the stock WebAssembly layer grows the memory by exactly that.
//! A browser's `memory.grow` takes time that rises with the memory's current
//! size, so a heap that climbs to gigabytes in 64 KiB steps spends most of its
//! loading time growing. This layer grows by an eighth of the current memory
//! (at least 16 MiB) and hands dlmalloc the whole grown region, so the number
//! of grows stays logarithmic in the heap size. The region is contiguous with
//! the previous one, which dlmalloc merges into its top chunk; the only memory
//! held beyond the heap is the unused tail of the last grow.

use core::alloc::{GlobalAlloc, Layout};
use core::arch::wasm32;
use core::cell::UnsafeCell;
use core::ptr;

#[cfg(target_feature = "atomics")]
compile_error!("the browser heap has no lock; a threaded build needs one");

const PAGE: usize = 64 * 1024;
/// The smallest grow, in pages (16 MiB).
const MIN_GROW_PAGES: usize = 256;

struct GeometricGrowth;

unsafe impl dlmalloc::Allocator for GeometricGrowth {
    fn alloc(&self, size: usize) -> (*mut u8, usize, u32) {
        let needed = size.div_ceil(PAGE);
        let wanted = needed.max(wasm32::memory_size(0) / 8).max(MIN_GROW_PAGES);
        // Near the memory's limit the larger grow can fail where the needed
        // one still fits.
        for pages in [wanted, needed] {
            let previous = wasm32::memory_grow(0, pages);
            if previous != usize::MAX {
                return region(previous, pages);
            }
            if pages == needed {
                break;
            }
        }
        (ptr::null_mut(), 0, 0)
    }

    fn remap(&self, _ptr: *mut u8, _old: usize, _new: usize, _can_move: bool) -> *mut u8 {
        ptr::null_mut()
    }

    fn free_part(&self, _ptr: *mut u8, _old: usize, _new: usize) -> bool {
        false
    }

    fn free(&self, _ptr: *mut u8, _size: usize) -> bool {
        false
    }

    fn can_release_part(&self, _flags: u32) -> bool {
        false
    }

    fn allocates_zeros(&self) -> bool {
        true
    }

    fn page_size(&self) -> usize {
        PAGE
    }
}

/// The grown pages as a dlmalloc region. A region that ends at the top of the
/// 32-bit address space keeps its last bytes out, so no pointer past its end
/// wraps to zero.
fn region(previous: usize, pages: usize) -> (*mut u8, usize, u32) {
    let base = previous * PAGE;
    let size = pages * PAGE;
    if base.wrapping_add(size) == 0 {
        return (base as *mut u8, size - 16, 0);
    }
    (base as *mut u8, size, 0)
}

struct Heap(UnsafeCell<dlmalloc::Dlmalloc<GeometricGrowth>>);

// The browser build has one thread (no `atomics` target feature), so the heap
// is never entered from two threads.
unsafe impl Sync for Heap {}

#[global_allocator]
static HEAP: Heap = Heap(UnsafeCell::new(dlmalloc::Dlmalloc::new_with_allocator(
    GeometricGrowth,
)));

unsafe impl GlobalAlloc for Heap {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        unsafe { (*self.0.get()).malloc(layout.size(), layout.align()) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        unsafe { (*self.0.get()).calloc(layout.size(), layout.align()) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { (*self.0.get()).free(ptr, layout.size(), layout.align()) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        unsafe { (*self.0.get()).realloc(ptr, layout.size(), layout.align(), new_size) }
    }
}
