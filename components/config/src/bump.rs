//! A bump allocator that reuses its memory once every allocation is freed.
//!
//! Each call into the component frees everything it allocates by the time the call is complete:
//! the arguments and the imported values are dropped while the call runs, and the result is freed
//! by the post-return function after the caller has copied it. Memory is handed out by bumping a
//! pointer, and once the count of live allocations reaches zero the pointer moves back to the
//! start. If anything were kept across calls the count wouldn't reach zero, and memory would grow
//! rather than being reused while still in use.

use core::alloc::{GlobalAlloc, Layout};
use core::arch::wasm32;
use core::cell::Cell;
use core::ptr;

const PAGE_SIZE: usize = 65536;

pub struct BumpAllocator {
    /// Where allocations start, the end of memory when first used, zero until then.
    start: Cell<usize>,
    /// Where the next allocation starts.
    next: Cell<usize>,
    /// The end of memory.
    end: Cell<usize>,
    /// The number of allocations not yet freed.
    live: Cell<usize>,
}

// wasm32-unknown-unknown is single threaded
unsafe impl Sync for BumpAllocator {}

impl BumpAllocator {
    pub const fn new() -> Self {
        Self {
            start: Cell::new(0),
            next: Cell::new(0),
            end: Cell::new(0),
            live: Cell::new(0),
        }
    }
}

unsafe impl GlobalAlloc for BumpAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if self.start.get() == 0 {
            // memory past its initial size isn't used by the stack or static data
            let end = wasm32::memory_size(0) * PAGE_SIZE;
            self.start.set(end);
            self.next.set(end);
            self.end.set(end);
        }

        let align = layout.align();
        let Some(addr) = self
            .next
            .get()
            .checked_add(align - 1)
            .map(|n| n & !(align - 1))
        else {
            return ptr::null_mut();
        };
        let Some(next) = addr.checked_add(layout.size()) else {
            return ptr::null_mut();
        };
        if next > self.end.get() {
            let pages = (next - self.end.get()).div_ceil(PAGE_SIZE);
            if wasm32::memory_grow(0, pages) == usize::MAX {
                return ptr::null_mut();
            }
            self.end.set(self.end.get() + pages * PAGE_SIZE);
        }

        self.next.set(next);
        self.live.set(self.live.get() + 1);
        addr as *mut u8
    }

    unsafe fn dealloc(&self, _ptr: *mut u8, _layout: Layout) {
        let live = self.live.get() - 1;
        self.live.set(live);
        if live == 0 {
            self.next.set(self.start.get());
        }
    }
}
