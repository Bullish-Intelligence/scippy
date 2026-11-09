//! Diagnostics and the outcome counts of an indexing run.

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

/// Counts and diagnostics from one indexing run.
#[derive(Clone, Debug, Default)]
pub struct IndexReport {
    pub files_discovered: usize,
    pub files_indexed: usize,
    pub declarations: usize,
    pub references_resolved: usize,
    pub references_unresolved: usize,
    pub references_ambiguous: usize,
    pub diagnostics: Vec<Diagnostic>,
}
