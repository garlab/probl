//! Scoped allocation measurement for this single-threaded benchmark only.
//! Fixtures and result serialization live outside the measurement. Timing uses
//! separate runs with accounting disabled. Includes overlapping reallocations.
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering::Relaxed};

pub struct Counting;
static ENABLED: AtomicBool = AtomicBool::new(false);
static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

fn add(bytes: usize) {
    let now = LIVE.fetch_add(bytes, Relaxed) + bytes;
    PEAK.fetch_max(now, Relaxed);
}

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() && ENABLED.load(Relaxed) {
            add(layout.size());
        }
        ptr
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) };
        if ENABLED.load(Relaxed) {
            LIVE.fetch_sub(layout.size(), Relaxed);
        }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        if !ENABLED.load(Relaxed) {
            return unsafe { System.realloc(ptr, layout, size) };
        }
        // Expose both buffers in PEAK even if the system could grow in place.
        let new_layout = Layout::from_size_align(size, layout.align()).unwrap();
        let new = unsafe { self.alloc(new_layout) };
        if !new.is_null() {
            unsafe { std::ptr::copy_nonoverlapping(ptr, new, layout.size().min(size)) };
            unsafe { self.dealloc(ptr, layout) };
        }
        new
    }
}

pub fn live() -> usize {
    LIVE.load(Relaxed)
}

pub fn measure(f: impl FnOnce() -> usize) -> (usize, usize) {
    LIVE.store(0, Relaxed);
    PEAK.store(0, Relaxed);
    assert!(!ENABLED.swap(true, Relaxed));
    let retained = f();
    ENABLED.store(false, Relaxed);
    assert_eq!(
        LIVE.load(Relaxed),
        0,
        "measured allocations must all be dropped in the scope"
    );
    (retained, PEAK.load(Relaxed))
}
