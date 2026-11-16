//! The values that the read API returns.
//!
//! A record holds paths and symbol strings. It never holds a database id.

use scippy_core::{Bound, ReferenceKind, ScippyError};

/// Reads a stored bound. An unknown text is a storage error.
pub(crate) fn parse_bound(text: &str) -> Result<Bound, ScippyError> {
    Bound::from_code(text).ok_or_else(|| {
        ScippyError::Storage(format!("unknown Bound value {text:?} in the database"))
    })
}

/// Reads a stored reference kind. An unknown text is a storage error.
pub(crate) fn parse_reference_kind(text: &str) -> Result<ReferenceKind, ScippyError> {
    ReferenceKind::from_code(text).ok_or_else(|| {
        ScippyError::Storage(format!(
            "unknown ReferenceKind value {text:?} in the database"
        ))
    })
}

macro_rules! text_enum {
    ($(#[$meta:meta])* $name:ident { $($variant:ident => $text:literal),+ $(,)? }) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        #[non_exhaustive]
        pub enum $name { $($variant),+ }

        impl $name {
            /// The text that the database stores.
            pub fn as_str(self) -> &'static str {
                match self { $(Self::$variant => $text),+ }
            }

            pub(crate) fn parse(text: &str) -> Result<Self, ScippyError> {
                match text {
                    $($text => Ok(Self::$variant),)+
                    other => Err(ScippyError::Storage(format!(
                        "unknown {} value {other:?} in the database", stringify!($name)
                    ))),
                }
            }
        }
    };
}

text_enum! {
    /// What the run did with a file.
    FileStatus {
        Indexed => "indexed",
        Skipped => "skipped",
        Failed => "failed",
        Shadowed => "shadowed",
    }
}

text_enum! {
    /// The kind of an edge.
    EdgeKind {
        Import => "import",
        Inheritance => "inheritance",
        Override => "override",
        Alias => "alias",
        Link => "link",
        Embed => "embed",
    }
}

text_enum! {
    /// The kind of a file-to-symbol relation.
    RelationKind {
        Import => "import",
        Use => "use",
        Link => "link",
        Embed => "embed",
    }
}

text_enum! {
    /// The role of an occurrence.
    OccurrenceRole {
        Definition => "definition",
        Reference => "reference",
        Import => "import",
        Read => "read",
    }
}

text_enum! {
    /// The result of the search for a reference, without its data.
    OutcomeKind {
        Resolved => "resolved",
        Ambiguous => "ambiguous",
        Unresolved => "unresolved",
        Bounded => "bounded",
    }
}

/// A symbol, named in a way that holds across revisions.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SymbolRef {
    /// The SCIP symbol string.
    pub symbol: String,
    /// The path of the document. It is set for a `local N` symbol, which is unique only inside
    /// one document.
    pub file: Option<String>,
}

impl SymbolRef {
    /// A symbol that is the same in every document.
    pub fn global(symbol: impl Into<String>) -> Self {
        Self {
            symbol: symbol.into(),
            file: None,
        }
    }

    /// A `local N` symbol of one document.
    pub fn local(symbol: impl Into<String>, file: impl Into<String>) -> Self {
        Self {
            symbol: symbol.into(),
            file: Some(file.into()),
        }
    }
}

/// The root set of the database.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct RootSetInfo {
    /// The SHA-256 of the project root.
    pub key: String,
    pub project_root: String,
    /// The shards of the latest run, sorted. The project root itself is not in the list.
    pub shards: Vec<String>,
}

/// The identity of one published revision.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct RevisionInfo {
    pub id: i64,
    pub input_digest: String,
    pub tool_version: String,
    pub input_complete: bool,
    pub analysis_exact: bool,
    pub strict: bool,
    pub scip_sha256: String,
    pub created_at: String,
    /// The size of the SCIP file in bytes.
    pub scip_size: u64,
    /// The unit of every span. It is `utf8-byte`.
    pub position_encoding: String,
}

/// One row of the input manifest.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct FileRecord {
    pub path: String,
    pub status: FileStatus,
    pub reason: Option<String>,
    pub pack: Option<String>,
    pub sha256: Option<String>,
    pub size: Option<u64>,
    /// The file on disk differed from the indexed content at the end of the run. The facts
    /// come from the indexed content. The file is not lost, and `input_complete` ignores it.
    pub stale: bool,
    /// The language of the file, when a pack set it.
    pub language: Option<String>,
    /// The path that the run read, when the file lies outside the project root.
    pub real_path: Option<String>,
}

/// A support file that the run read, such as `pyproject.toml`. It is part of the input.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct InputRecord {
    /// The path below the project root, or the virtual path of a file in a shard.
    pub path: String,
    /// The path that the run read, when the file lies outside the project root.
    pub real_path: Option<String>,
    /// The SHA-256 at the start of the run. `None` when the file did not exist.
    pub sha256: Option<String>,
}

/// An occurrence. The span is in UTF-8 bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct OccurrenceRecord {
    pub span: (u32, u32),
    pub symbol: SymbolRef,
    pub role: OccurrenceRole,
}

/// The result of the search for one reference, as the database stores it. It matches
/// `scippy_core::ReferenceOutcome`, but its targets are [`SymbolRef`] values, not symbol strings.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum StoredOutcome {
    Resolved(SymbolRef),
    /// The candidates, in the order of their symbol strings.
    Ambiguous(Vec<SymbolRef>),
    /// The reason code.
    Unresolved(String),
    /// The bound that stopped the search.
    Bounded(String),
}

/// A reference, resolved or not.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ReferenceRecord {
    pub path: String,
    pub span: (u32, u32),
    pub kind: Option<ReferenceKind>,
    pub outcome: StoredOutcome,
    /// For a resolved reference: the bound that stopped the search.
    pub limited: Option<Bound>,
}

/// Which references to read. A field that is `None` does not filter.
#[derive(Clone, Debug, Default)]
pub struct ReferenceQuery {
    pub path: Option<String>,
    pub outcome: Option<OutcomeKind>,
}

/// An edge between two symbols, or from a source site.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct EdgeRecord {
    pub kind: EdgeKind,
    /// The file of the source site.
    pub path: String,
    pub source: Option<SymbolRef>,
    pub target: Option<SymbolRef>,
    pub span: Option<(u32, u32)>,
    pub link_kind: Option<String>,
    pub label: Option<String>,
    pub state: Option<String>,
    pub anchor_state: Option<String>,
    /// For an ambiguous link: the candidates, in the order of their symbol strings.
    pub candidates: Vec<SymbolRef>,
    /// The symbol of the heading or block part of a link, when the part resolves.
    pub anchor: Option<SymbolRef>,
}

/// A diagnostic of the run.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct DiagnosticRecord {
    pub severity: String,
    pub stage: String,
    pub code: String,
    pub message: String,
    pub path: Option<String>,
    pub span: Option<(u32, u32)>,
}

/// One source location that supports a relation.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct SupportRecord {
    pub path: String,
    pub span: (u32, u32),
    pub kind: RelationKind,
    pub limited: Option<Bound>,
}

/// The source text of a file, checked against the stored hash.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Source {
    /// The file has the stored SHA-256.
    Verified(String),
    /// The file exists and has other bytes.
    Changed,
    /// The file does not exist.
    Missing,
}
