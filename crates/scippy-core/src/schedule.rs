//! The scheduler of the shards: run jobs on worker threads under a memory budget.

use std::collections::VecDeque;
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::sync::{Condvar, Mutex, MutexGuard, PoisonError};
use std::thread;

/// The stack of a worker. The main thread has 8 MiB. The workers need no less.
pub(crate) const WORKER_STACK_BYTES: usize = 256 << 20;

/// The memory budget that a run uses when the caller gives none: 50 GiB.
pub(crate) const DEFAULT_MEMORY_BUDGET: u64 = 50 << 30;

/// The number of workers that a run uses when the caller gives none.
pub(crate) fn default_jobs() -> usize {
    thread::available_parallelism().map_or(1, |n| n.get())
}

/// The number of threads that resolve the references of one shard when the caller gives none.
///
/// At most `min(jobs, shards)` shards run at once. Each of them gets the same share of `jobs`,
/// so the shard threads times the resolve threads stay at or under `jobs`. A share is at least 1.
pub(crate) fn default_resolve_threads(jobs: usize, shards: usize) -> usize {
    let at_once = jobs.max(1).min(shards.max(1));
    (jobs / at_once).max(1)
}

/// Lock a mutex and recover the guard when another thread panicked while it held the lock.
/// A worker panic is not lost: `run_ordered` raises it again after all jobs end. The data that
/// the locks guard stays valid, because no code that can panic runs while a lock is held.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

struct Queue<J> {
    /// The jobs that wait, the largest estimate first.
    waiting: VecDeque<(usize, u64, J)>,
    /// The sum of the estimates of the running jobs.
    running: u64,
    /// The number of running jobs.
    active: usize,
}

/// Run every job and return the results in the order of `jobs`.
///
/// A job is an estimate of its peak memory in bytes and a payload. With one worker, the jobs
/// run in order on the calling thread. With more, the workers take the jobs from the largest
/// estimate down. A worker starts a job only when the estimates of the running jobs and of this
/// job together stay under `budget`. A job over the budget runs alone.
/// Each worker makes one state with `init` and gives it to every job that it runs.
pub(crate) fn run_ordered<J, S, T>(
    jobs: Vec<(u64, J)>,
    workers: usize,
    budget: u64,
    init: impl Fn() -> S + Sync,
    work: impl Fn(&mut S, u64, J) -> T + Sync,
) -> Vec<T>
where
    J: Send,
    T: Send,
{
    let workers = workers.max(1).min(jobs.len());
    if workers <= 1 {
        let mut state = init();
        return jobs
            .into_iter()
            .map(|(estimate, job)| work(&mut state, estimate, job))
            .collect();
    }
    let count = jobs.len();
    let mut waiting: Vec<(usize, u64, J)> = jobs
        .into_iter()
        .enumerate()
        .map(|(n, (estimate, job))| (n, estimate, job))
        .collect();
    // The sort is stable, so equal estimates keep the order of the jobs.
    waiting.sort_by_key(|&(_, estimate, _)| std::cmp::Reverse(estimate));
    let queue = Mutex::new(Queue {
        waiting: waiting.into(),
        running: 0,
        active: 0,
    });
    let changed = Condvar::new();
    type Outcome<T> = Option<thread::Result<T>>;
    let results: Mutex<Vec<Outcome<T>>> = Mutex::new((0..count).map(|_| None).collect());
    thread::scope(|scope| {
        for _ in 0..workers {
            let spawned = thread::Builder::new()
                .stack_size(WORKER_STACK_BYTES)
                .spawn_scoped(scope, || {
                    let mut state = init();
                    loop {
                        let (n, estimate, job) = {
                            let mut q = lock(&queue);
                            loop {
                                let Some(&(_, estimate, _)) = q.waiting.front() else {
                                    return;
                                };
                                if q.active == 0 || q.running + estimate <= budget {
                                    break;
                                }
                                q = changed.wait(q).unwrap_or_else(PoisonError::into_inner);
                            }
                            let Some((n, estimate, job)) = q.waiting.pop_front() else {
                                return;
                            };
                            q.running += estimate;
                            q.active += 1;
                            (n, estimate, job)
                        };
                        let outcome =
                            catch_unwind(AssertUnwindSafe(|| work(&mut state, estimate, job)));
                        lock(&results)[n] = Some(outcome);
                        let mut q = lock(&queue);
                        q.running -= estimate;
                        q.active -= 1;
                        changed.notify_all();
                    }
                });
            // A worker that cannot start leaves its jobs to the others.
            if spawned.is_err() {
                break;
            }
        }
    });
    results
        .into_inner()
        .unwrap_or_else(PoisonError::into_inner)
        .into_iter()
        .map(|outcome| match outcome.expect("every job ran") {
            Ok(value) => value,
            Err(panic) => resume_unwind(panic),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    #[test]
    fn the_resolve_threads_share_the_jobs_between_the_shards() {
        // One shard gets every job.
        assert_eq!(default_resolve_threads(16, 1), 16);
        // Fewer shards than jobs: the share rounds down.
        assert_eq!(default_resolve_threads(16, 3), 5);
        assert_eq!(default_resolve_threads(16, 4), 4);
        // At least as many shards as jobs: one thread for each shard.
        assert_eq!(default_resolve_threads(16, 16), 1);
        assert_eq!(default_resolve_threads(16, 100), 1);
        assert_eq!(default_resolve_threads(1, 5), 1);
        // The shard threads times the resolve threads never exceed the jobs.
        for jobs in 1..40 {
            for shards in 0..60 {
                let at_once = jobs.min(shards.max(1));
                assert!(at_once * default_resolve_threads(jobs, shards) <= jobs);
            }
        }
    }

    #[test]
    fn the_default_budget_is_50_gib() {
        assert_eq!(DEFAULT_MEMORY_BUDGET, 50 << 30);
    }

    #[test]
    fn results_keep_the_order_of_the_jobs() {
        let jobs: Vec<(u64, usize)> = (0..20).map(|n| ((n as u64 * 7) % 5, n)).collect();
        let out = run_ordered(jobs, 4, u64::MAX, || (), |_, _, n| n * 2);
        assert_eq!(out, (0..20).map(|n| n * 2).collect::<Vec<_>>());
    }

    #[test]
    fn running_estimates_stay_under_the_budget() {
        let now = AtomicU64::new(0);
        let peak = AtomicU64::new(0);
        let jobs: Vec<(u64, ())> = (0..12).map(|n| (10 + n % 3, ())).collect();
        run_ordered(
            jobs,
            6,
            25,
            || (),
            |_, estimate, ()| {
                let at = now.fetch_add(estimate, Ordering::SeqCst) + estimate;
                peak.fetch_max(at, Ordering::SeqCst);
                thread::sleep(std::time::Duration::from_millis(5));
                now.fetch_sub(estimate, Ordering::SeqCst);
            },
        );
        assert!(peak.load(Ordering::SeqCst) <= 25);
    }

    #[test]
    fn a_job_over_the_budget_runs_alone() {
        let now = AtomicU64::new(0);
        let jobs = vec![(100u64, ()), (5, ()), (5, ())];
        run_ordered(
            jobs,
            3,
            20,
            || (),
            |_, estimate, ()| {
                let at = now.fetch_add(estimate, Ordering::SeqCst) + estimate;
                if estimate == 100 {
                    assert_eq!(at, 100);
                }
                thread::sleep(std::time::Duration::from_millis(5));
                now.fetch_sub(estimate, Ordering::SeqCst);
            },
        );
    }
}
