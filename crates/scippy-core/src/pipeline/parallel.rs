//! Load the files of a shard on worker threads.
//!
//! A file builds without any other file, so workers parse the files and build the graphs of
//! the files that the graph cache lacks. This thread owns the engine and the cache. It looks
//! the files up in the cache, adds each graph to the engine in file order, and stores the new
//! graphs in the cache in file order. The engine ends up as a serial load leaves it, so the
//! output does not depend on the number of threads. A failure of a file gives the message and
//! the diagnostic of the serial load.

use super::Doc;
use super::load::{Parsed, finish_file, parse_file};
use super::stack::ShardContext;
use crate::bindings::{BindingEngine, BuildFailure, BuiltFile, CompiledRules, FileBuilder, Lookup};
use crate::cache::CachedGraph;
use crate::report::IndexReport;
use crate::schedule::WORKER_STACK_BYTES;
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::{Mutex, PoisonError};
use std::thread;

/// The most graphs that this thread holds from the cache, before the engine adds them.
const HIT_WINDOW: usize = 256;

/// The least number of files that the lookup runs ahead of the merge.
const AHEAD_MIN: usize = 16;

/// The number of files that the lookup runs ahead of the merge, for `threads` build threads.
/// The files in this window are queued, built, or finished. A finished graph waits for the
/// merge. Without a window, up to all files wait at once, and each graph takes 20 MiB or more.
/// A window of four files for each thread keeps the workers busy. The window is never empty, so
/// the file that the merge needs is always in it.
fn ahead(threads: usize) -> usize {
    threads.saturating_mul(4).max(AHEAD_MIN)
}

/// What the parse of a file gives: the report lines, and the parsed file when it did not skip.
struct Front {
    report: IndexReport,
    parsed: Option<Parsed>,
}

/// A file for a worker to build.
struct Task {
    at: usize,
    dialect: usize,
    text: std::sync::Arc<str>,
    globals: Vec<(String, String)>,
}

type Built = Result<BuiltFile, BuildFailure>;
type Key = Option<(String, String)>;

/// Where a file stands in the merge.
enum Slot {
    /// The lookup did not reach the file.
    Idle,
    /// The parse skipped the file.
    Skip,
    /// The cache has the graph.
    Hit(CachedGraph),
    /// A worker builds the graph. `Key` is the cache key.
    Queued(Key),
    /// The graph is ready.
    Built(Key, Built),
}

/// Run `task` for each index below `count` on up to `threads` threads, and return the results in
/// the order of the indexes. A panic in a task stops the other threads from taking new indexes,
/// and this function raises the panic again after the threads end.
fn parallel_map<T: Send>(count: usize, threads: usize, task: impl Fn(usize) -> T + Sync) -> Vec<T> {
    let next = AtomicUsize::new(0);
    let stop = AtomicBool::new(false);
    let slots: Vec<Mutex<Option<thread::Result<T>>>> =
        (0..count).map(|_| Mutex::new(None)).collect();
    thread::scope(|scope| {
        for _ in 0..threads.min(count) {
            let spawned = thread::Builder::new()
                .stack_size(WORKER_STACK_BYTES)
                .spawn_scoped(scope, || {
                    loop {
                        let n = next.fetch_add(1, Ordering::Relaxed);
                        if n >= count || stop.load(Ordering::Relaxed) {
                            return;
                        }
                        let out = catch_unwind(AssertUnwindSafe(|| task(n)));
                        if out.is_err() {
                            stop.store(true, Ordering::Relaxed);
                        }
                        *slots[n].lock().unwrap_or_else(PoisonError::into_inner) = Some(out);
                    }
                });
            // A thread that cannot start leaves its files to the others.
            if spawned.is_err() {
                break;
            }
        }
    });
    slots
        .into_iter()
        .enumerate()
        .map(|(n, slot)| {
            let done = slot.into_inner().unwrap_or_else(PoisonError::into_inner);
            match done {
                Some(Ok(value)) => value,
                Some(Err(panic)) => resume_unwind(panic),
                // No thread took the index: the threads did not start.
                None => task(n),
            }
        })
        .collect()
}

/// Add the report lines of a worker to the report of the shard.
fn merge_parse_report(into: &mut IndexReport, from: IndexReport) {
    into.files_skipped += from.files_skipped;
    into.files_shadowed += from.files_shadowed;
    into.lost_files.extend(from.lost_files);
    into.diagnostics.extend(from.diagnostics);
}

/// Load the files on `threads` worker threads, and add them to the engine in the order of `jobs`.
///
/// Step 1: the workers parse the files. Step 2: this thread looks up each file in the graph
/// cache, and sends the files that miss to the workers, which build them. This thread adds each
/// graph to the engine in file order, and stores each new graph in the cache.
pub(super) fn load_parallel(
    cx: &ShardContext,
    compiled: &CompiledRules,
    engine: &mut BindingEngine,
    jobs: &[(usize, bool)],
    threads: usize,
    report: &mut IndexReport,
) -> Vec<(usize, Doc)> {
    let mut fronts: Vec<Option<Front>> = parallel_map(jobs.len(), threads, |n| {
        let mut scratch = IndexReport::default();
        let parsed = parse_file(
            cx.analysis,
            cx.extractors,
            cx.model,
            cx.store,
            cx.files[jobs[n].0],
            &mut scratch,
        );
        Front {
            report: scratch,
            parsed,
        }
    })
    .into_iter()
    .map(Some)
    .collect();
    let builder = FileBuilder::new(cx.analysis, compiled);
    let abort = AtomicBool::new(false);
    let (task_sender, task_receiver) = mpsc::channel::<(usize, Task)>();
    let task_receiver = Mutex::new(task_receiver);
    let (done_sender, done_receiver) = mpsc::channel::<(usize, thread::Result<Built>)>();
    let mut docs = Vec::new();
    let mut panic = None;
    thread::scope(|scope| {
        for _ in 0..threads.min(jobs.len()) {
            let done_sender = done_sender.clone();
            let (builder, abort, task_receiver) = (&builder, &abort, &task_receiver);
            let spawned = thread::Builder::new()
                .stack_size(WORKER_STACK_BYTES)
                .spawn_scoped(scope, move || {
                    loop {
                        let next = task_receiver
                            .lock()
                            .unwrap_or_else(PoisonError::into_inner)
                            .recv();
                        let Ok((n, task)) = next else {
                            return;
                        };
                        if abort.load(Ordering::Relaxed) {
                            return;
                        }
                        let built = catch_unwind(AssertUnwindSafe(|| {
                            builder.build(
                                task.dialect,
                                &cx.files[task.at].path,
                                &task.text,
                                &task.globals,
                            )
                        }));
                        if done_sender.send((n, built)).is_err() {
                            return;
                        }
                    }
                });
            // A thread that cannot start leaves its files to the others.
            if spawned.is_err() {
                break;
            }
        }
        drop(done_sender);
        // The guard stops the workers when this thread ends the loop, and when it panics.
        struct StopOnDrop<'a>(&'a AtomicBool);
        impl Drop for StopOnDrop<'_> {
            fn drop(&mut self) {
                self.0.store(true, Ordering::Relaxed);
            }
        }
        let _stop = StopOnDrop(&abort);
        let mut slots: Vec<Slot> = jobs.iter().map(|_| Slot::Idle).collect();
        let mut looked_up = 0;
        let mut held = 0;
        let mut merged = 0;
        let ahead = ahead(threads);
        while merged < jobs.len() {
            // Look up more files. Two windows bound the graphs that wait for the merge: the hits
            // that this thread holds, and the files between the merge and the lookup.
            while looked_up < jobs.len() && held < HIT_WINDOW && looked_up - merged < ahead {
                let n = looked_up;
                looked_up += 1;
                let (at, _) = jobs[n];
                let Some(parsed) = fronts[n].as_ref().and_then(|f| f.parsed.as_ref()) else {
                    slots[n] = Slot::Skip;
                    continue;
                };
                let path = &cx.files[at].path;
                let globals = cx.model.resolver_globals(path);
                slots[n] = match engine.lookup(parsed.dialect(), path, parsed.text(), &globals) {
                    Lookup::Hit(cached) => {
                        held += 1;
                        Slot::Hit(cached)
                    }
                    Lookup::Miss { key } => {
                        let task = Task {
                            at,
                            dialect: parsed.dialect(),
                            text: parsed.shared_text(),
                            globals,
                        };
                        // The send fails only when no worker lives. This thread then builds the
                        // file when it needs the graph.
                        let _ = task_sender.send((n, task));
                        Slot::Queued(key)
                    }
                };
            }
            if matches!(slots[merged], Slot::Queued(_)) {
                match done_receiver.recv() {
                    Ok((n, Ok(built))) => place(&mut slots, n, built),
                    Ok((_, Err(payload))) => {
                        abort.store(true, Ordering::Relaxed);
                        panic = Some(payload);
                        break;
                    }
                    Err(_) => {
                        // No worker lives. Build the file here.
                        let (at, _) = jobs[merged];
                        let parsed = fronts[merged].as_ref().and_then(|f| f.parsed.as_ref());
                        let parsed = parsed.expect("a queued file has a parse");
                        let built = catch_unwind(AssertUnwindSafe(|| {
                            builder.build(
                                parsed.dialect(),
                                &cx.files[at].path,
                                parsed.text(),
                                &cx.model.resolver_globals(&cx.files[at].path),
                            )
                        }));
                        match built {
                            Ok(built) => place(&mut slots, merged, built),
                            Err(payload) => {
                                abort.store(true, Ordering::Relaxed);
                                panic = Some(payload);
                                break;
                            }
                        }
                    }
                }
                continue;
            }
            let (at, resolve) = jobs[merged];
            let file = cx.files[at];
            let slot = std::mem::replace(&mut slots[merged], Slot::Idle);
            let front = fronts[merged].take().expect("each file merges once");
            let mut scratch = IndexReport::default();
            let target = if resolve { &mut *report } else { &mut scratch };
            merge_parse_report(target, front.report);
            if let Some(parsed) = front.parsed {
                let added = match slot {
                    Slot::Hit(cached) => {
                        held -= 1;
                        engine.add_cached(&file.path, &cached, resolve)
                    }
                    Slot::Built(key, Ok(built)) => {
                        engine.add_built(&file.path, &built, key.as_ref(), resolve)
                    }
                    Slot::Built(_, Err(failure)) => Err(failure),
                    Slot::Idle | Slot::Skip | Slot::Queued(_) => {
                        unreachable!("a parsed file has a graph before the merge")
                    }
                };
                if let Some(doc) = finish_file(file, parsed, added, target)
                    && resolve
                {
                    docs.push((at, doc));
                }
            }
            merged += 1;
        }
        drop(task_sender);
    });
    if let Some(payload) = panic {
        resume_unwind(payload);
    }
    docs
}

/// Put the graph that a worker built into the slot of its file.
fn place(slots: &mut [Slot], n: usize, built: Built) {
    let Slot::Queued(key) = std::mem::replace(&mut slots[n], Slot::Idle) else {
        unreachable!("a worker built a file that no one queued");
    };
    slots[n] = Slot::Built(key, built);
}

#[cfg(test)]
mod tests {
    use super::{AHEAD_MIN, ahead};

    #[test]
    fn the_window_is_four_files_for_each_thread_and_at_least_the_minimum() {
        assert_eq!(ahead(1), AHEAD_MIN);
        assert_eq!(ahead(4), AHEAD_MIN);
        assert_eq!(ahead(5), 20);
        assert_eq!(ahead(8), 32);
        assert_eq!(ahead(64), 256);
        assert_eq!(ahead(usize::MAX), usize::MAX);
        // The merge needs the file at the head, so the window cannot be empty.
        assert!(ahead(0) >= 1);
    }
}
