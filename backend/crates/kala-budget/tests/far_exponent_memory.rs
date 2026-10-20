//! Verifies that exponents differing by millions never allocate a power of
//! ten of that size: peak allocation during far-apart arithmetic stays tiny.
//!
//! The test binary defines a tracking global allocator so it can assert the
//! peak heap use directly. It intentionally contains a single test so no other
//! test allocates concurrently.

use kala_budget::decimal::{Decimal, DecimalError};
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

static TRACKING: AtomicBool = AtomicBool::new(false);
static CURRENT: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

struct TrackingAllocator;

impl TrackingAllocator {
    fn record(size: usize) {
        if TRACKING.load(Ordering::Relaxed) {
            let current = CURRENT.fetch_add(size, Ordering::Relaxed) + size;
            PEAK.fetch_max(current, Ordering::Relaxed);
        }
    }

    fn release(size: usize) {
        if TRACKING.load(Ordering::Relaxed) {
            CURRENT.fetch_sub(size, Ordering::Relaxed);
        }
    }
}

unsafe impl GlobalAlloc for TrackingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { System.alloc(layout) };
        if !pointer.is_null() {
            Self::record(layout.size());
        }
        pointer
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { System.alloc_zeroed(layout) };
        if !pointer.is_null() {
            Self::record(layout.size());
        }
        pointer
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        Self::release(layout.size());
        unsafe { System.dealloc(pointer, layout) };
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let new_pointer = unsafe { System.realloc(pointer, layout, new_size) };
        if !new_pointer.is_null() {
            if new_size >= layout.size() {
                Self::record(new_size - layout.size());
            } else {
                Self::release(layout.size() - new_size);
            }
        }
        new_pointer
    }
}

#[global_allocator]
static ALLOCATOR: TrackingAllocator = TrackingAllocator;

#[test]
fn far_apart_exponents_stay_bounded() {
    TRACKING.store(true, Ordering::Relaxed);
    CURRENT.store(0, Ordering::Relaxed);
    PEAK.store(0, Ordering::Relaxed);
    let started = Instant::now();

    let high = Decimal::parse("1e999999").expect("parse");
    let low = Decimal::parse("1e-999999").expect("parse");
    let sum = high.add(&low).expect("sum");
    assert_eq!(
        sum.to_python_string(),
        "1.000000000000000000000000000E+999999"
    );
    assert!(sum.digit_count() <= 28);

    let beyond = Decimal::parse("1e999999999").expect("parse");
    let below = Decimal::parse("1e-999999999").expect("parse");
    assert_eq!(beyond.add(&below).unwrap_err(), DecimalError::Overflow);
    assert_eq!(
        beyond.sub(&beyond).expect("cancel").to_python_string(),
        "0E+999999"
    );
    let doubled = below.add(&below).expect("underflow");
    assert_eq!(doubled.to_python_string(), "0E-1000026");

    // A million-digit exponent gap with large (but bounded) coefficients.
    let wide_high = Decimal::parse("123456789012345678901234567890e970000").expect("parse");
    let wide_low = Decimal::parse("987654321098765432109876543210e-1000000").expect("parse");
    let wide_sum = wide_high.add(&wide_low).expect("sum");
    assert!(wide_sum.digit_count() <= 28);

    // Repair round 2: values constructed at CPython's extreme construction
    // bounds stay exact on storage and underflow in bounded memory.
    let lowest = Decimal::parse("1e-1999999999999999997").expect("parse");
    let next_lowest = Decimal::parse("1e-1999999999999999996").expect("parse");
    assert_eq!(lowest.exponent(), -1_999_999_999_999_999_997);
    assert_eq!(
        lowest.add(&lowest).expect("underflow").to_python_string(),
        "0E-1000026"
    );
    assert_eq!(
        next_lowest
            .sub(&lowest)
            .expect("underflow")
            .to_python_string(),
        "0E-1000026"
    );
    // A large coefficient at the extreme bound must not allocate a power of
    // ten for the context alignment either.
    let huge_coefficient = format!("{}e-1999999999999999997", "1".repeat(2_000));
    let huge = Decimal::parse(&huge_coefficient).expect("parse");
    assert_eq!(
        huge.add(&Decimal::zero())
            .expect("underflow")
            .to_python_string(),
        "0E-1000026"
    );

    let elapsed = started.elapsed();
    TRACKING.store(false, Ordering::Relaxed);
    let peak = PEAK.load(Ordering::Relaxed);
    assert!(
        peak < 1 << 20,
        "peak allocation {peak} bytes exceeds the 1 MiB bound"
    );
    assert!(
        elapsed < Duration::from_secs(5),
        "far-apart arithmetic took {elapsed:?}"
    );
}
