//! The structured analysis of one run. The crate `scippy-metalayer` stores it.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::bindings::Bound;
use crate::emit::{self, IndexData, Role, SymbolInfo};
use crate::error::ScippyError;
use crate::language::ReferenceKind;
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
    /// True when the file on disk differs, at the end of the run, from `source`. The facts come
    /// from `source`, which the run read once. The file is not lost.
    #[serde(default)]
    pub stale: bool,
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
    /// The bound that stopped the search of a resolved reference. The result is the nearest
    /// binding, and other ends can exist. It is `None` for any other outcome. A bounded
    /// reference names its bound in the outcome. The text form is `work`, `queue-length`,
    /// `arena-cells`, or `failed`.
    #[serde(default)]
    pub limited: Option<Bound>,
    /// How the reference names its target in the source. It is `None` for a reference of a
    /// native pack, which has no such distinction. The text form is `plain`, `member`, or
    /// `keyword`.
    #[serde(default)]
    pub kind: Option<ReferenceKind>,
}

impl AnalysisReference {
    /// `native` is true for a reference that a native pack gave.
    pub(crate) fn from_reconciled(r: &Reconciled, native: bool) -> Self {
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
            limited: r.limited,
            kind: (!native).then_some(r.kind),
        }
    }

    /// True when a search stopped at a limit: the outcome is bounded, or it keeps the nearest
    /// binding of a stopped search.
    pub fn is_inexact(&self) -> bool {
        self.limited.is_some() || matches!(self.outcome, ReferenceOutcome::Bounded(_))
    }
}

/// An edge that a declaration or a link adds to the references: an inheritance edge, or a link
/// or an embed of a native pack. A reference carries the target. An edge names its references
/// by span, and the store joins them.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnalysisEdge {
    /// `inheritance`, `link`, or `embed`.
    pub kind: String,
    /// The file of the source site.
    pub path: String,
    /// For an inheritance edge: the class symbol.
    pub source: Option<String>,
    /// The span of the edge: the supertype name, or the whole link.
    pub span: (u32, u32),
    /// The span of the reference that names the target. It is `None` when the edge has none.
    pub reference_span: Option<(u32, u32)>,
    /// The span of the reference that names the anchor of a link, such as the heading of `[[t#S]]`.
    pub anchor_span: Option<(u32, u32)>,
    /// The kind of link in the language of the pack, such as `wikilink`.
    pub link_kind: Option<String>,
    pub label: Option<String>,
    /// For a link: `resolved`, `missing`, or `ambiguous`.
    pub state: Option<String>,
    /// For a link: `resolved`, `missing`, or `none`.
    pub anchor_state: Option<String>,
}

/// What the pipeline captures for a snapshot.
#[derive(Default)]
pub(crate) struct Capture {
    pub references: Vec<AnalysisReference>,
    pub edges: Vec<AnalysisEdge>,
}

/// A file that the run accepted and did not index.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnalysisFile {
    pub path: String,
    /// The id of the language pack that accepts the file, when one pack does.
    pub pack: Option<String>,
    /// Where the run read a file that lies outside the project root.
    pub real_path: Option<String>,
    /// `skipped`, `failed`, or `shadowed`. It follows from `reason`: see [`file_status`].
    pub status: String,
    /// The code of the diagnostic that decided the status.
    pub reason: String,
    /// The SHA-256 of the bytes that the run read, when it read them.
    pub sha256: Option<String>,
}

/// The status of a file that the run did not index, from the code that decided it.
///
/// - `shadowed`: another file shadows it, so no import reaches it. This is legal Python.
/// - `failed`: Scippy accepted the file and an operation failed.
/// - `skipped`: Scippy does not accept the file.
pub fn file_status(reason: &str) -> &'static str {
    match reason {
        "shadowed-file" => "shadowed",
        "unreadable-file" | "syntax-error" | "bind-error" => "failed",
        _ => "skipped",
    }
}

/// The diagnostic codes that make a snapshot incomplete, although their severity is a warning.
/// Each code names input that the run lost without a file row: a manifest, a support file,
/// a dependency, a file over a size limit that a pack found, or a path that has no valid name.
pub const INCOMPLETE_CODES: [&str; 6] = [
    "support-file-error",
    "manifest-error",
    "root-name-collision",
    "file-too-large",
    "non-utf8-path",
    "invalid-path",
];

/// The facts and diagnostics of one completed indexing run.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnalysisSnapshot {
    pub documents: Vec<AnalysisDocument>,
    /// The accepted files that the run did not index, sorted by path. A file here can still have
    /// a document, as a note that the run could not read has a document with no facts but its
    /// module.
    #[serde(default)]
    pub lost_files: Vec<AnalysisFile>,
    pub references: Vec<AnalysisReference>,
    /// The inheritance edges, and the links and embeds of native packs, sorted.
    #[serde(default)]
    pub edges: Vec<AnalysisEdge>,
    pub external_symbols: Vec<AnalysisSymbol>,
    pub diagnostics: Vec<AnalysisDiagnostic>,
    /// False when the run lost input. It is true only when no file has the status `skipped` or
    /// `failed`, no diagnostic has the severity `Error`, and no diagnostic has a code in
    /// [`INCOMPLETE_CODES`]. A shadowed file does not change it. A bounded search does not
    /// change it: see `analysis_exact`.
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
        capture: Capture,
        report: &IndexReport,
        context: RunContext,
    ) -> Result<Self, ScippyError> {
        let stale: std::collections::HashSet<&str> =
            report.stale_files.iter().map(String::as_str).collect();
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
                    source: d.source.to_string(),
                    stale: stale.contains(d.path.as_str()),
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
        let Capture {
            mut references,
            mut edges,
        } = capture;
        references.sort_by(|a, b| (&a.path, a.start, a.end).cmp(&(&b.path, b.start, b.end)));
        edges.sort_by(|a, b| {
            (&a.path, a.span, &a.kind, &a.source).cmp(&(&b.path, b.span, &b.kind, &b.source))
        });
        let bounded_references = references.iter().filter(|r| r.is_inexact()).count();
        let lost_files: Vec<AnalysisFile> = report
            .lost_files
            .iter()
            .map(|l| AnalysisFile {
                path: l.path.clone(),
                pack: l.pack.clone(),
                real_path: l.real_path.clone(),
                status: file_status(l.reason).to_owned(),
                reason: l.reason.to_owned(),
                sha256: l.sha256.clone(),
            })
            .collect();
        let input_complete = !lost_files.iter().any(|f| f.status != "shadowed")
            && !report.diagnostics.iter().any(|d| {
                d.severity == crate::Severity::Error || INCOMPLETE_CODES.contains(&d.code)
            });
        Ok(Self {
            documents,
            lost_files,
            references,
            edges,
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
            input_complete,
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

    fn reference(outcome: ReferenceOutcome, limited: Option<Bound>) -> AnalysisReference {
        AnalysisReference {
            path: "a.py".into(),
            start: 0,
            end: 1,
            outcome,
            limited,
            kind: None,
        }
    }

    #[test]
    fn a_bounded_search_makes_the_reference_inexact() {
        let bounded = reference(ReferenceOutcome::Bounded("work".into()), None);
        let kept = reference(ReferenceOutcome::Resolved("s".into()), Some(Bound::Work));
        let exact = reference(ReferenceOutcome::Resolved("s".into()), None);
        assert!(bounded.is_inexact());
        assert!(kept.is_inexact());
        assert!(!exact.is_inexact());
    }

    #[test]
    fn the_enums_keep_their_stable_text_in_json() {
        let mut r = reference(
            ReferenceOutcome::Resolved("s".into()),
            Some(Bound::QueueLength),
        );
        r.kind = Some(ReferenceKind::Keyword);
        let json = serde_json::to_string(&r).unwrap();
        assert!(json.contains(r#""limited":"queue-length""#), "{json}");
        assert!(json.contains(r#""kind":"keyword""#), "{json}");
        let back: AnalysisReference = serde_json::from_str(&json).unwrap();
        assert_eq!(back, r);
        let unknown = json.replace("queue-length", "no-such-bound");
        assert!(serde_json::from_str::<AnalysisReference>(&unknown).is_err());
    }

    #[test]
    fn the_hash_is_lowercase_hex_sha256() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
