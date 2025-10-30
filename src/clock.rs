use std::sync::atomic::{AtomicU64, Ordering};

#[repr(usize)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Timer {
    DecodeFrame = 0,
    DecodeCheckpoint,
    DecodeStatestream,
    EncodeFrame,
    EncodeCheckpoint,
    EncodeStatestream,
    Count,
}
#[repr(usize)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Counter {
    EncReusedBlocks = 0,
    EncReusedSuperblocks,
    EncSkippedBlocks,
    EncMemCmps,
    EncHashes,
    EncTotalBlocks,
    EncTotalSuperblocks,
    EncTotalKBsIn,
    EncTotalKBsOut,
    Count,
}
static TIME_ACC: [AtomicU64; Timer::Count as usize] = [
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
];
static TIME_COUNTS: [AtomicU64; Timer::Count as usize] = [
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
];
static COUNTS: [AtomicU64; Counter::Count as usize] = [
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
];

pub struct Stopwatch(Timer, std::time::Instant);
impl Stopwatch {
    fn new(t: Timer) -> Self {
        Self(t, std::time::Instant::now())
    }
}
impl Drop for Stopwatch {
    fn drop(&mut self) {
        TIME_ACC[self.0 as usize].fetch_add(
            u64::try_from(self.1.elapsed().as_micros()).unwrap_or(u64::MAX),
            Ordering::Relaxed,
        );
        TIME_COUNTS[self.0 as usize].fetch_add(1, Ordering::Relaxed);
    }
}

pub fn time(t: Timer) -> Stopwatch {
    Stopwatch::new(t)
}
pub fn count(c: Counter, amt: u64) -> u64 {
    COUNTS[c as usize].fetch_add(amt, Ordering::Relaxed) + amt
}
pub struct Times {
    pub count: u64,
    pub micros: u64,
}
pub fn stats(t: Timer) -> Times {
    Times {
        count: TIME_COUNTS[t as usize].load(Ordering::Relaxed),
        micros: TIME_ACC[t as usize].load(Ordering::Relaxed),
    }
}
pub fn counts(c: Counter) -> u64 {
    COUNTS[c as usize].load(Ordering::Relaxed)
}
