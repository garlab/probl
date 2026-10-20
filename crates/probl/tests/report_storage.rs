//! Memory is part of the report contract: reading text, JSON or a numeric
//! summary must not eagerly materialize every formatted outcome probability.
use probl::Options;
#[cfg(feature = "json")]
use probl::ReportJsonLimits;
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

struct Counting;
static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);
static ALLOCATED: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() {
            let live = LIVE.fetch_add(layout.size(), Ordering::Relaxed) + layout.size();
            PEAK.fetch_max(live, Ordering::Relaxed);
            ALLOCATED.fetch_add(layout.size(), Ordering::Relaxed);
        }
        ptr
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
        unsafe { System.dealloc(ptr, layout) };
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        let ptr = unsafe { System.realloc(ptr, layout, size) };
        if !ptr.is_null() {
            LIVE.fetch_add(size, Ordering::Relaxed);
            let live = LIVE.fetch_sub(layout.size(), Ordering::Relaxed) - layout.size();
            PEAK.fetch_max(live, Ordering::Relaxed);
            ALLOCATED.fetch_add(size, Ordering::Relaxed);
        }
        ptr
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

// One test in this binary keeps unrelated tests out of allocation measurements.
#[test]
fn exact_populations_are_shared_and_formatted_tables_are_created_on_demand() {
    let program = probl::compile("storage", "for n in 1..20000 {report n by 0}").unwrap();
    let before = LIVE.load(Ordering::Relaxed);
    let outcome = program.run(&Options::new()).unwrap();
    assert!(outcome.text().contains("median"));
    #[cfg(feature = "json")]
    {
        let document = outcome.reports_json(&ReportJsonLimits::default()).unwrap();
        assert!(document.contains("10000.5"));
    }
    let retained = LIVE.load(Ordering::Relaxed).saturating_sub(before);
    // Ample fixed slack for the runner and test harness, but not space for a
    // full Quantity/Estimate + formatted String per retained value.
    assert!(
        retained < 80 * 20_000 + 1_048_576,
        "retained {retained} bytes before requesting a table"
    );
    let before = ALLOCATED.load(Ordering::Relaxed);
    let reports = vec![outcome.reports()[0].clone(); 8];
    let clones = ALLOCATED.load(Ordering::Relaxed) - before;
    assert!(
        clones < 128 * 1024,
        "cloning reports copied {clones} bytes of population data"
    );
    drop(outcome);

    // The cloned report owns its population, and several first readers safely
    // share the one cached slice after the original outcome has gone away.
    let barrier = std::sync::Barrier::new(reports.len());
    let pointers = std::thread::scope(|scope| {
        let handles: Vec<_> = reports
            .iter()
            .map(|report| {
                let barrier = &barrier;
                scope.spawn(move || {
                    barrier.wait();
                    let group = &report.groups()[0];
                    let values = group.distribution().unwrap();
                    assert_eq!(values.len(), 20_000);
                    assert_eq!(values.first().unwrap().0, "1");
                    assert_eq!(values.last().unwrap().0, "20,000");
                    for (_, estimate) in values {
                        assert!((estimate.point().unwrap() - 1.0 / 20_000.0).abs() < 1e-15);
                        assert!(estimate.is_complete() && estimate.sampling().is_none());
                    }
                    assert_eq!(group.numeric().unwrap().quantile(0.5).unwrap().point(), Some(10000.5));
                    values.as_ptr() as usize
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect::<Vec<_>>()
    });
    assert!(pointers.iter().all(|p| *p == pointers[0]));
    drop(reports);
    // Fixed groups and batch size must bound collection memory as trials grow.
    let program = probl::compile("moments storage", "let x~uniform(0,1); report x by 0").unwrap();
    let mut limits = probl::Limits::default();
    limits.max_threads = 1;
    let options = Options::new()
        .seed(42)
        .limits(limits)
        .report_collection(0, probl::ReportCollection::Moments);
    drop(program.run(&options.clone().runs(1000)).unwrap());
    let peaks: Vec<_> = [2000, 20000]
        .into_iter()
        .map(|runs| {
            let before = LIVE.load(Ordering::Relaxed);
            PEAK.store(before, Ordering::Relaxed);
            let outcome = program.run(&options.clone().runs(runs)).unwrap();
            let peak = PEAK.load(Ordering::Relaxed).saturating_sub(before);
            let retained = LIVE.load(Ordering::Relaxed).saturating_sub(before);
            assert!(retained < 128 * 1024, "{runs} trials retained {retained} bytes");
            assert!(outcome.reports()[0].groups()[0].distribution().is_none());
            peak
        })
        .collect();
    assert!(
        peaks[1] < 2 * peaks[0] + 64 * 1024,
        "ten times as many trials grew collection memory: {peaks:?}"
    );
}
