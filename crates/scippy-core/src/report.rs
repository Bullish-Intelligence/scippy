//! Diagnostics and the outcome counts of an indexing run.

use std::collections::BTreeMap;
use std::fmt;

/// How serious a diagnostic is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
    Info,
}

/// The pipeline stage that raised a diagnostic.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    Discover,
    Parse,
    Extract,
    Bind,
    Reconcile,
    Identity,
    Emit,
}

/// A non-fatal problem. The index can still be valid.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    pub severity: Severity,
    pub stage: Stage,
    /// Project-relative path of the file, when the problem belongs to one file.
    pub file: Option<String>,
    /// Byte span in the file, when the problem belongs to one span.
    pub range: Option<(u32, u32)>,
    /// Stable machine-readable identifier, for example `symlink-skipped`.
    pub code: &'static str,
    pub message: String,
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let severity = match self.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
            Severity::Info => "info",
        };
        write!(f, "{severity}[{}]", self.code)?;
        if let Some(file) = &self.file {
            write!(f, " {file}")?;
        }
        write!(f, ": {}", self.message)
    }
}

/// A file that the run accepted for indexing and did not index.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LostFile {
    /// The project-relative path.
    pub path: String,
    /// The id of the language pack that accepts the file, when one pack does.
    pub pack: Option<String>,
    /// Where the run read a file that lies outside the project root.
    pub real_path: Option<String>,
    /// The code of the diagnostic that decided the loss.
    pub reason: &'static str,
    /// The SHA-256 of the bytes that the run read, when it read them.
    pub sha256: Option<String>,
}

/// What the policy made of the references that had two or more raw ends.
///
/// The count is per unique reference location. A raw end is a definition node
/// that the engine reached, before the reconciler joins it to a declaration.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MultiTargetStats {
    /// Locations with two or more distinct raw ends, before the shadow filter.
    pub raw: usize,
    /// Resolved after the shadow filter dropped ends. A shadowed path loses to a path of higher
    /// precedence, for example a parameter that hides an outer name.
    pub shadow_filtered: usize,
    /// Resolved because one end joined to a declaration. The others were bindings,
    /// such as an import alias.
    pub alias_collapsed: usize,
    /// Resolved because one end joined to a declaration and another end is outside the
    /// project, for example a builtin. A local definition shadows the builtin.
    pub builtin_shadow: usize,
    /// Resolved because the language pack refused some ends as targets of the reference kind.
    pub pack_refused: usize,
    /// Resolved because the nearest-binding rule dropped value-flow ends.
    pub nearest_binding: usize,
    /// Resolved because several declarations share one symbol. An example is a rebound variable.
    pub same_symbol: usize,
    /// Two or more distinct symbols. No target.
    pub ambiguous: usize,
    /// No end joined to a declaration, or the symbols belong to another document. No target.
    pub unresolved: usize,
    /// A work bound stopped the search. No target.
    pub bounded: usize,
}

/// Reference locations that a bound stopped, by kind of bound.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BoundedCounts {
    /// The search used all of `max_work`.
    pub work: usize,
    /// The queue of paths grew over `max_queue_len`.
    pub queue_length: usize,
    /// The search added more than `max_arena_cells` cells to the path arenas.
    pub arena_cells: usize,
    /// The search failed or panicked.
    pub failed: usize,
}

/// The cost of the searches of one run, from the counters of the stitcher.
///
/// The counts hold no time. They are the same for each run of the same input and the same
/// limits, with any number of jobs: sums and maxima do not depend on the order of the searches.
/// A search that failed or panicked gives no counters, so it is not in these counts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SearchWork {
    /// The number of searches that returned counters. One search runs for each reference node.
    pub searches: usize,
    /// The work of all searches, added. One unit is one path that a search extends or one
    /// candidate that it tries.
    pub work_total: usize,
    /// The work of the largest search.
    pub work_max: usize,
    /// The longest queue of paths that waited for extension in one search.
    pub queue_length_max: usize,
    /// The most cells that one search added to the path arenas.
    pub arena_cells_max: usize,
}

impl SearchWork {
    /// Count one search with its counters.
    pub fn record(&mut self, work: usize, queue_length: usize, arena_cells: usize) {
        self.searches += 1;
        self.work_total += work;
        self.work_max = self.work_max.max(work);
        self.queue_length_max = self.queue_length_max.max(queue_length);
        self.arena_cells_max = self.arena_cells_max.max(arena_cells);
    }

    /// Add the counts of another set of searches: sum the totals, take the maxima.
    pub fn merge(&mut self, other: &SearchWork) {
        self.searches += other.searches;
        self.work_total += other.work_total;
        self.work_max = self.work_max.max(other.work_max);
        self.queue_length_max = self.queue_length_max.max(other.queue_length_max);
        self.arena_cells_max = self.arena_cells_max.max(other.arena_cells_max);
    }
}

/// Counts and diagnostics from one indexing run.
///
/// The reference counts are per unique location in a document.
/// A location holds one or more reference nodes of the engine.
#[derive(Clone, Debug, Default)]
pub struct IndexReport {
    /// The run had a graph cache (`IndexRequest::graph_cache`).
    pub graph_cache_enabled: bool,
    /// File graphs loaded from the cache.
    pub graph_cache_hits: usize,
    /// File graphs built while the cache was enabled.
    pub graph_cache_misses: usize,
    /// Cache failures in the run, such as a lock timeout. The first failure turns the cache
    /// off. Each failure waits at most the cache timeout.
    pub graph_cache_failures: usize,
    pub files_discovered: usize,
    /// Files that became documents.
    pub files_indexed: usize,
    /// Files that the run read or tried to read, and then skipped.
    pub files_skipped: usize,
    /// Files that another file shadows, so that no import reaches them. They are legal Python
    /// and do not count as skipped.
    pub files_shadowed: usize,
    /// Files that a pack accepts and the walker skipped before it read them: a symbolic link,
    /// a file over the size limit, a bad file name, or a file after the total limit.
    pub discovery_skipped: usize,
    pub declarations: usize,
    /// Occurrences written to the index.
    pub occurrences: usize,
    pub references_resolved: usize,
    /// Resolved references whose target is an external symbol, such as a builtin.
    /// The count is a part of `references_resolved`.
    pub references_external: usize,
    pub references_unresolved: usize,
    pub references_ambiguous: usize,
    /// References whose search hit a work bound. They have no target.
    pub references_bounded: usize,
    /// The bounded references by kind of bound. The counts add up to `references_bounded`.
    pub bounded_by_kind: BoundedCounts,
    /// The cost of the searches that resolved the references.
    pub search_work: SearchWork,
    /// Why the unresolved references have no target, by stable reason code.
    pub unresolved_reasons: BTreeMap<&'static str, usize>,
    /// The `syntax_type` of the resolver ends that joined no declaration, with counts.
    pub unjoined_end_kinds: BTreeMap<String, usize>,
    pub multi_target: MultiTargetStats,
    pub diagnostics: Vec<Diagnostic>,
    /// Every accepted file that the run did not index, with its reason. A file skipped at
    /// discovery is here too. The list is complete after the run.
    pub lost_files: Vec<LostFile>,
    /// The indexed files whose content on disk differs, at the end of the run, from the content
    /// that the run read once and indexed. The facts of such a file stay. Sorted by path.
    pub stale_files: Vec<String>,
}

impl fmt::Display for IndexReport {
    /// The text summary of the run: counts, the multi-target line, and the cache line. It ends
    /// with no new line. It holds no diagnostics: print each [`Diagnostic`] on its own.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "files discovered: {}", self.files_discovered)?;
        writeln!(f, "files indexed: {}", self.files_indexed)?;
        writeln!(f, "files skipped: {}", self.files_skipped)?;
        writeln!(f, "files shadowed: {}", self.files_shadowed)?;
        writeln!(f, "declarations: {}", self.declarations)?;
        writeln!(f, "occurrences: {}", self.occurrences)?;
        writeln!(f, "references resolved: {}", self.references_resolved)?;
        writeln!(f, "references external: {}", self.references_external)?;
        writeln!(f, "references unresolved: {}", self.references_unresolved)?;
        writeln!(f, "references ambiguous: {}", self.references_ambiguous)?;
        write!(f, "references bounded: {}", self.references_bounded)?;
        let m = &self.multi_target;
        write!(
            f,
            "\nmulti-target: raw {} -> shadow-filtered {}, pack-refused {}, nearest-binding {}, alias-collapsed {}, builtin-shadow {}, same-symbol {}, ambiguous {}, unresolved {}, bounded {}",
            m.raw,
            m.shadow_filtered,
            m.pack_refused,
            m.nearest_binding,
            m.alias_collapsed,
            m.builtin_shadow,
            m.same_symbol,
            m.ambiguous,
            m.unresolved,
            m.bounded
        )?;
        let b = &self.bounded_by_kind;
        write!(
            f,
            "\nbounded by kind: work {}, queue-length {}, arena-cells {}, failed {}",
            b.work, b.queue_length, b.arena_cells, b.failed
        )?;
        for (reason, n) in &self.unresolved_reasons {
            write!(f, "\nunresolved {reason}: {n}")?;
        }
        for (kind, n) in &self.unjoined_end_kinds {
            write!(f, "\nunjoined end {kind}: {n}")?;
        }
        if self.graph_cache_enabled {
            write!(
                f,
                "\nfile graphs: {} cached, {} built",
                self.graph_cache_hits, self.graph_cache_misses
            )?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_search_gives_zero_counts() {
        let work = SearchWork::default();
        assert_eq!(work.searches, 0);
        assert_eq!(work.work_total, 0);
        assert_eq!(work.work_max, 0);
        assert_eq!(work.queue_length_max, 0);
        assert_eq!(work.arena_cells_max, 0);
        let mut empty = SearchWork::default();
        empty.merge(&SearchWork::default());
        assert_eq!(empty, SearchWork::default());
    }

    #[test]
    fn record_sums_the_work_and_keeps_the_maxima() {
        let mut work = SearchWork::default();
        work.record(10, 3, 100);
        work.record(40, 2, 50);
        work.record(5, 9, 70);
        assert_eq!(work.searches, 3);
        assert_eq!(work.work_total, 55);
        assert_eq!(work.work_max, 40);
        assert_eq!(work.queue_length_max, 9);
        assert_eq!(work.arena_cells_max, 100);
        assert!(work.work_max <= work.work_total);
    }

    #[test]
    fn merge_is_the_same_as_recording_every_search_in_any_order() {
        let searches = [(10, 3, 100), (40, 2, 50), (5, 9, 70), (0, 0, 0)];
        let part = |items: &[(usize, usize, usize)]| {
            let mut work = SearchWork::default();
            for &(w, q, a) in items {
                work.record(w, q, a);
            }
            work
        };
        let all = part(&searches);
        let (first, second) = (part(&searches[..1]), part(&searches[1..]));
        let mut forward = first;
        forward.merge(&second);
        let mut backward = second;
        backward.merge(&first);
        assert_eq!(forward, all);
        assert_eq!(backward, all);
    }

    #[test]
    fn display_prints_the_summary_lines() {
        let mut report = IndexReport {
            files_discovered: 3,
            references_bounded: 2,
            ..IndexReport::default()
        };
        report.unresolved_reasons.insert("no-definition", 4);
        let text = report.to_string();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines[0], "files discovered: 3");
        assert_eq!(lines[10], "references bounded: 2");
        assert!(lines[11].starts_with("multi-target: raw 0 -> shadow-filtered 0"));
        assert!(lines[12].starts_with("bounded by kind: work 0"));
        assert_eq!(lines[13], "unresolved no-definition: 4");
        assert!(!text.contains("file graphs"));
        assert!(!text.ends_with('\n'));
        report.graph_cache_enabled = true;
        report.graph_cache_hits = 1;
        assert!(
            report
                .to_string()
                .ends_with("file graphs: 1 cached, 0 built")
        );
    }
}
