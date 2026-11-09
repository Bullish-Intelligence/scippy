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

/// Counts and diagnostics from one indexing run.
///
/// The reference counts are per unique location in a document.
/// A location holds one or more reference nodes of the engine.
#[derive(Clone, Debug, Default)]
pub struct IndexReport {
    pub files_discovered: usize,
    /// Files that became documents.
    pub files_indexed: usize,
    /// Files that the run read or tried to read, and then skipped.
    pub files_skipped: usize,
    pub declarations: usize,
    /// Occurrences written to the index.
    pub occurrences: usize,
    pub references_resolved: usize,
    pub references_unresolved: usize,
    pub references_ambiguous: usize,
    /// References whose search hit a work bound. They have no target.
    pub references_bounded: usize,
    /// Why the unresolved references have no target, by stable reason code.
    pub unresolved_reasons: BTreeMap<&'static str, usize>,
    /// The `syntax_type` of the resolver ends that joined no declaration, with counts.
    pub unjoined_end_kinds: BTreeMap<String, usize>,
    pub multi_target: MultiTargetStats,
    pub diagnostics: Vec<Diagnostic>,
}
