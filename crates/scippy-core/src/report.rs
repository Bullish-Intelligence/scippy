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

/// Counts and diagnostics from one indexing run.
///
/// The reference counts are per unique location in a document.
/// A location holds one or more reference nodes of the engine.
#[derive(Clone, Debug, Default)]
pub struct IndexReport {
    /// File graphs loaded from the cache.
    pub graph_cache_hits: usize,
    /// File graphs built while the cache was enabled.
    pub graph_cache_misses: usize,
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
    /// Why the unresolved references have no target, by stable reason code.
    pub unresolved_reasons: BTreeMap<&'static str, usize>,
    /// The `syntax_type` of the resolver ends that joined no declaration, with counts.
    pub unjoined_end_kinds: BTreeMap<String, usize>,
    pub multi_target: MultiTargetStats,
    pub diagnostics: Vec<Diagnostic>,
    /// Every accepted file that the run did not index, with its reason. A file skipped at
    /// discovery is here too. The list is complete after the run.
    pub lost_files: Vec<LostFile>,
}
