//! The memory estimate and the memory meter of a shard.

/// The estimate of the peak memory of a shard is a fixed part for the builtins and the rules,
/// and a part that grows with the source bytes that the engine loads. The loaded bytes are
/// the files of the shard and the files that it imports from other shards.
/// The growing part has a ceiling: the peak of the largest shards (mypy, pydantic-ai, pynix)
/// stays near 4 to 8 GB, while the peak of a small shard of dense code reaches 2.8 GB for
/// each MB. The ceiling is above the largest measured peak, 8.08 GB (llama-infernal).
/// `scripts/oracle/RESULTS.md` has the measurements.
pub(super) const SHARD_BASE_BYTES: u64 = 300 << 20;

pub(super) const SHARD_BYTES_PER_SOURCE_BYTE: u64 = 3000;

pub(super) const SHARD_GRAPH_CEILING_BYTES: u64 = 10 << 30;

/// The growing part rises with the build threads of the shard. Each build worker holds the graph
/// of a file in a stack graph of its own, and the finished graphs wait for the merge in file
/// order. The peak memory of mypy (one shard, `--resolve-threads 8`) was 4.0 GiB with one build
/// thread, 4.8 GiB with 2, 6.7 GiB with 4, and 7.8 GiB with 8. The growing part, in permille,
/// is 1000 + 250 for each build thread after the first, up to 2000. This gives 1.25, 1.75, and
/// 2.0 times for 2, 4, and 8 threads, and the measured ratios are 1.21, 1.68, and 1.95.
pub(super) const BUILD_THREAD_PERMILLE: u64 = 250;

pub(super) const BUILD_THREAD_MAX_PERMILLE: u64 = 2000;

/// The estimate of the peak memory of a shard that loads `source_bytes` of source and builds its
/// graphs on `build_threads` threads.
pub(super) fn shard_estimate(source_bytes: u64, build_threads: usize) -> u64 {
    let extra = (build_threads.max(1) as u64 - 1).saturating_mul(BUILD_THREAD_PERMILLE);
    let permille = 1000u64.saturating_add(extra).min(BUILD_THREAD_MAX_PERMILLE);
    let growing = SHARD_BYTES_PER_SOURCE_BYTE
        .saturating_mul(source_bytes)
        .min(SHARD_GRAPH_CEILING_BYTES);
    SHARD_BASE_BYTES + (growing as u128 * permille as u128 / 1000) as u64
}

/// The resident memory of the process in KiB now, or 0 when `/proc` has no figure.
pub(super) fn resident_memory() -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|status| {
            status
                .lines()
                .find_map(|line| line.strip_prefix("VmRSS:"))
                .and_then(|rest| rest.split_whitespace().next()?.parse().ok())
        })
        .unwrap_or(0)
}

/// Measures the resident memory of the process while a shard runs. A thread samples it, so the
/// meter never touches the high-water mark of the process (`VmHWM`, `ru_maxrss`).
/// The figure is for the whole process. It is exact for one shard only with `--jobs 1`.
pub(super) struct MemoryMeter {
    start: u64,
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    sampler: std::thread::JoinHandle<u64>,
}

impl MemoryMeter {
    pub(super) fn start() -> Self {
        use std::sync::atomic::{AtomicBool, Ordering};
        let stop = std::sync::Arc::new(AtomicBool::new(false));
        let flag = std::sync::Arc::clone(&stop);
        let start = resident_memory();
        let sampler = std::thread::spawn(move || {
            let mut peak = start;
            while !flag.load(Ordering::Relaxed) {
                peak = peak.max(resident_memory());
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            peak.max(resident_memory())
        });
        MemoryMeter {
            start,
            stop,
            sampler,
        }
    }

    /// The resident memory in KiB at the start, and the largest figure that the sampler saw.
    pub(super) fn finish(self) -> (u64, u64) {
        self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
        let peak = self.sampler.join().unwrap_or(self.start);
        (self.start, peak.max(self.start))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_estimate_of_a_shard_follows_the_formula_and_the_cap() {
        assert_eq!(shard_estimate(0, 1), 300 << 20);
        assert_eq!(shard_estimate(1_000_000, 1), (300 << 20) + 3_000_000_000);
        // The measured peaks: llama-infernal 8.08 GB from 2.74 MB of source.
        assert!(shard_estimate(2_740_000, 1) >= 8_085_428 * 1024);
        // The ceiling is above the largest measured peak, and it holds for any size.
        let ceiling = shard_estimate(u64::MAX, 1);
        assert_eq!(ceiling, (300 << 20) + (10 << 30));
        assert!(ceiling > 8_085_428 * 1024 + (1 << 30));
        assert_eq!(shard_estimate(1 << 40, 1), ceiling);
    }

    #[test]
    fn build_threads_raise_the_growing_part_up_to_twice() {
        let growing = |threads| shard_estimate(1_000_000, threads) - SHARD_BASE_BYTES;
        assert_eq!(growing(0), 3_000_000_000);
        assert_eq!(growing(1), 3_000_000_000);
        assert_eq!(growing(2), 3_750_000_000);
        assert_eq!(growing(4), 5_250_000_000);
        assert_eq!(growing(8), 6_000_000_000);
        assert_eq!(growing(1000), 6_000_000_000);
        // The ceiling rises with the threads too, and it never overflows.
        assert_eq!(
            shard_estimate(u64::MAX, 8),
            SHARD_BASE_BYTES + 2 * SHARD_GRAPH_CEILING_BYTES
        );
        assert!(shard_estimate(u64::MAX, usize::MAX) > 0);
    }

    #[test]
    fn the_memory_meter_reports_a_peak_not_below_the_start() {
        let meter = MemoryMeter::start();
        let block = vec![1u8; 64 << 20];
        std::thread::sleep(std::time::Duration::from_millis(50));
        let (start, peak) = meter.finish();
        assert!(block.iter().all(|&b| b == 1));
        if start > 0 {
            assert!(peak >= start);
        }
    }
}
