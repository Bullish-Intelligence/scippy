//! The structured analysis of one run. The crate `scippy-metalayer` stores it.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::emit::{self, IndexData, Role, SymbolInfo};
use crate::error::ScippyError;
use crate::reconcile::{BindingOutcome, Reconciled, UnresolvedReason};
use crate::report::IndexReport;

/// A document and its indexed occurrences.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnalysisDocument {
    pub path: String,
    pub language: String,
    /// The id of the language pack that indexed the file. It is `None` for a stub document.
    #[serde(default)]
    pub pack: Option<String>,
    /// Where the run read a file that lies outside the project root. `None` for a file below it.
    #[serde(default)]
    pub real_path: Option<String>,
    /// The SHA-256 of the bytes that the run read, as lowercase hex. It is `None` for a stub
    /// document, which has no text of its own.
    #[serde(default)]
    pub sha256: Option<String>,
    pub source: String,
    pub occurrences: Vec<AnalysisOccurrence>,
    pub symbols: Vec<AnalysisSymbol>,
}

/// An indexed occurrence. Positions are byte offsets in the document source.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnalysisOccurrence {
    pub start: u32,
    pub end: u32,
    pub symbol: String,
    pub role: String,
}

/// Information about a declared symbol.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnalysisSymbol {
    pub symbol: String,
    pub kind: String,
    pub display_name: String,
    pub implements: Vec<String>,
    pub references: Vec<String>,
}

fn symbol_info(s: &SymbolInfo) -> AnalysisSymbol {
    AnalysisSymbol {
        symbol: s.symbol.as_str().to_owned(),
        kind: format!("{:?}", s.kind),
        display_name: s.display_name.clone(),
        implements: s.implements.iter().map(|s| s.as_str().to_owned()).collect(),
        references: s.references.iter().map(|s| s.as_str().to_owned()).collect(),
    }
}

/// A diagnostic with its machine-readable code and source range.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnalysisDiagnostic {
    pub severity: String,
    pub stage: String,
    pub file: Option<String>,
    pub range: Option<(u32, u32)>,
    pub code: String,
    pub message: String,
}

/// The result of one reference search.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReferenceOutcome {
    Resolved(String),
    Ambiguous(Vec<String>),
    Unresolved(String),
    Bounded(String),
}

/// A reference, including references that have no target.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnalysisReference {
    pub path: String,
    pub start: u32,
    pub end: u32,
    pub outcome: ReferenceOutcome,
    /// The bound that stopped the search of a resolved reference, as `work`, `queue-length`,
    /// `arena-cells`, or `failed`. The result is the nearest binding, and other ends can exist.
    /// It is `None` for any other outcome. A bounded reference names its bound in the outcome.
    #[serde(default)]
    pub limited: Option<String>,
}

impl AnalysisReference {
    pub(crate) fn from_reconciled(r: &Reconciled) -> Self {
        let outcome = match &r.outcome {
            BindingOutcome::Resolved(s) => ReferenceOutcome::Resolved(s.as_str().to_owned()),
            BindingOutcome::Ambiguous(s) => {
                ReferenceOutcome::Ambiguous(s.iter().map(|s| s.as_str().to_owned()).collect())
            }
            BindingOutcome::Unresolved(r) => ReferenceOutcome::Unresolved(
                match r {
                    UnresolvedReason::NoPath => "no-path",
                    UnresolvedReason::NoDeclaration => "no-declaration",
                    UnresolvedReason::NotAddressable => "not-addressable",
                    UnresolvedReason::NotTargetable => "not-targetable",
                }
                .to_owned(),
            ),
            BindingOutcome::Bounded(b) => ReferenceOutcome::Bounded(b.code().to_owned()),
        };
        Self {
            path: r.reference.path.as_str().to_owned(),
            start: r.reference.start_byte,
            end: r.reference.end_byte,
            outcome,
            limited: r.limited.map(|b| b.code().to_owned()),
        }
    }

    /// True when a search stopped at a limit: the outcome is bounded, or it keeps the nearest
    /// binding of a stopped search.
    pub fn is_inexact(&self) -> bool {
        self.limited.is_some() || matches!(self.outcome, ReferenceOutcome::Bounded(_))
    }
}

/// The diagnostic codes that make a snapshot incomplete, although their severity is a warning.
/// Each code names input that the run lost without a skipped file: a manifest, a support file,
/// a dependency, or a file over a size limit.
pub const INCOMPLETE_CODES: [&str; 4] = [
    "support-file-error",
    "manifest-error",
    "root-name-collision",
    "file-too-large",
];

/// The facts and diagnostics of one completed indexing run.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnalysisSnapshot {
    pub documents: Vec<AnalysisDocument>,
    pub references: Vec<AnalysisReference>,
    pub external_symbols: Vec<AnalysisSymbol>,
    pub diagnostics: Vec<AnalysisDiagnostic>,
    /// False when the run lost input. It is true only when the run skipped no file (at discovery
    /// or later), raised no error diagnostic, and raised no diagnostic with a code in
    /// [`INCOMPLETE_CODES`]. A bounded search does not change it: see `analysis_exact`.
    pub input_complete: bool,
    /// False when a reference search stopped at a limit. A reference with the outcome `Bounded`
    /// has no target. A resolved reference with a `limited` mark keeps the nearest binding.
    /// This field does not depend on `input_complete`.
    pub analysis_exact: bool,
    /// The count of references whose search stopped at a limit. It counts both kinds.
    #[serde(default)]
    pub bounded_references: usize,
    /// The SCIP index from the same run.
    pub scip: Vec<u8>,
    /// The version of the tool that made the run.
    #[serde(default)]
    pub tool_version: String,
    /// The canonical absolute path of the project root.
    #[serde(default)]
    pub project_root: String,
    /// The virtual folders that the run resolved as separate shards, sorted.
    #[serde(default)]
    pub shards: Vec<String>,
    /// The settings of the run that change the analysis: the input limits, the resolve limits,
    /// and the package overrides. It is text with a fixed order.
    #[serde(default)]
    pub options: String,
}

/// What the pipeline knows about a run, beyond the facts.
pub(crate) struct RunContext {
    pub project_root: String,
    pub shards: Vec<String>,
    pub options: String,
    /// For each document path: the pack id and the real path of a file outside the root.
    pub files: HashMap<String, (String, Option<String>)>,
}

/// The SHA-256 of `bytes` as lowercase hex.
pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    Sha256::digest(bytes)
        .iter()
        .fold(String::with_capacity(64), |mut out, b| {
            let _ = write!(out, "{b:02x}");
            out
        })
}

impl AnalysisSnapshot {
    pub(crate) fn new(
        data: &IndexData,
        mut references: Vec<AnalysisReference>,
        report: &IndexReport,
        context: RunContext,
    ) -> Result<Self, ScippyError> {
        let documents = data
            .documents
            .iter()
            .map(|d| {
                // A stub document has no language and no text.
                let stub = d.language.is_empty();
                let file = context.files.get(d.path.as_str());
                AnalysisDocument {
                    path: d.path.as_str().to_owned(),
                    language: d.language.clone(),
                    pack: file.map(|f| f.0.clone()),
                    real_path: file.and_then(|f| f.1.clone()),
                    sha256: (!stub).then(|| sha256_hex(d.source.as_bytes())),
                    source: d.source.clone(),
                    occurrences: d
                        .occurrences
                        .iter()
                        .map(|o| AnalysisOccurrence {
                            start: o.start_byte,
                            end: o.end_byte,
                            symbol: o.symbol.as_str().to_owned(),
                            role: match o.role {
                                Role::Reference => "reference",
                                Role::Definition => "definition",
                                Role::Import => "import",
                                Role::ReadAccess => "read",
                            }
                            .to_owned(),
                        })
                        .collect(),
                    symbols: d.symbols.iter().map(symbol_info).collect(),
                }
            })
            .collect();
        references.sort_by(|a, b| (&a.path, a.start, a.end).cmp(&(&b.path, b.start, b.end)));
        let bounded_references = references.iter().filter(|r| r.is_inexact()).count();
        Ok(Self {
            documents,
            references,
            external_symbols: data.external_symbols.iter().map(symbol_info).collect(),
            diagnostics: report
                .diagnostics
                .iter()
                .map(|d| AnalysisDiagnostic {
                    severity: format!("{:?}", d.severity),
                    stage: format!("{:?}", d.stage),
                    file: d.file.clone(),
                    range: d.range,
                    code: d.code.to_owned(),
                    message: d.message.clone(),
                })
                .collect(),
            input_complete: report.files_skipped == 0
                && report.discovery_skipped == 0
                && !report.diagnostics.iter().any(|d| {
                    d.severity == crate::Severity::Error || INCOMPLETE_CODES.contains(&d.code)
                }),
            analysis_exact: bounded_references == 0,
            bounded_references,
            scip: emit::encode(data.clone())?,
            tool_version: env!("CARGO_PKG_VERSION").to_owned(),
            project_root: context.project_root,
            shards: context.shards,
            options: context.options,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reference(outcome: ReferenceOutcome, limited: Option<&str>) -> AnalysisReference {
        AnalysisReference {
            path: "a.py".into(),
            start: 0,
            end: 1,
            outcome,
            limited: limited.map(str::to_owned),
        }
    }

    #[test]
    fn a_bounded_search_makes_the_reference_inexact() {
        let bounded = reference(ReferenceOutcome::Bounded("work".into()), None);
        let kept = reference(ReferenceOutcome::Resolved("s".into()), Some("work"));
        let exact = reference(ReferenceOutcome::Resolved("s".into()), None);
        assert!(bounded.is_inexact());
        assert!(kept.is_inexact());
        assert!(!exact.is_inexact());
    }

    #[test]
    fn the_hash_is_lowercase_hex_sha256() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
