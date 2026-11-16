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

/// The estimate of the peak memory of a shard that loads `source_bytes` of source.
pub(super) fn shard_estimate(source_bytes: u64) -> u64 {
    SHARD_BASE_BYTES
        + SHARD_BYTES_PER_SOURCE_BYTE
            .saturating_mul(source_bytes)
            .min(SHARD_GRAPH_CEILING_BYTES)
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
        assert_eq!(shard_estimate(0), 300 << 20);
        assert_eq!(shard_estimate(1_000_000), (300 << 20) + 3_000_000_000);
        // The measured peaks: llama-infernal 8.08 GB from 2.74 MB of source.
        assert!(shard_estimate(2_740_000) >= 8_085_428 * 1024);
        // The ceiling is above the largest measured peak, and it holds for any size.
        let ceiling = shard_estimate(u64::MAX);
        assert_eq!(ceiling, (300 << 20) + (10 << 30));
        assert!(ceiling > 8_085_428 * 1024 + (1 << 30));
        assert_eq!(shard_estimate(1 << 40), ceiling);
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
