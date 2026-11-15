//! Errors that stop an indexing run.

use std::fmt;
use std::io;
use std::path::PathBuf;

/// A fatal error. Non-fatal problems use [`crate::Diagnostic`] instead.
#[derive(Debug)]
pub enum ScippyError {
    /// The project root is missing, unreadable, or not a directory.
    InvalidRoot { path: PathBuf, reason: String },
    /// A document path breaks the SCIP path rules.
    InvalidPath { path: String, reason: &'static str },
    /// A symbol breaks the SCIP symbol grammar.
    InvalidSymbol { symbol: String, reason: String },
    /// A byte span does not fit the source text of its document.
    InvalidRange {
        path: String,
        start_byte: u32,
        end_byte: u32,
        reason: &'static str,
    },
    /// Two distinct declarations claim one symbol.
    SymbolCollision { symbol: String, detail: String },
    /// Two documents have one path.
    DuplicateDocument { path: String },
    /// A language pack is invalid: its query or its binding rules do not compile.
    Pack { pack: &'static str, reason: String },
    /// A file system operation failed.
    Io { path: PathBuf, source: io::Error },
    /// Protobuf serialization failed.
    Serialize(String),
    /// The graph cache or metalayer database failed.
    Storage(String),
    /// The input is incomplete, and a strict run refuses to go on.
    Incomplete,
    /// The metalayer database holds another root set. Use `--replace-root-set` to replace it.
    RootSetMismatch { stored: String, requested: String },
}

impl fmt::Display for ScippyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidRoot { path, reason } => {
                write!(f, "invalid project root {}: {reason}", path.display())
            }
            Self::InvalidPath { path, reason } => {
                write!(f, "invalid document path {path:?}: {reason}")
            }
            Self::InvalidSymbol { symbol, reason } => {
                write!(f, "invalid symbol {symbol:?}: {reason}")
            }
            Self::InvalidRange {
                path,
                start_byte,
                end_byte,
                reason,
            } => write!(
                f,
                "invalid range {start_byte}..{end_byte} in {path}: {reason}"
            ),
            Self::SymbolCollision { symbol, detail } => {
                write!(f, "symbol collision for {symbol:?}: {detail}")
            }
            Self::DuplicateDocument { path } => write!(f, "duplicate document {path}"),
            Self::Pack { pack, reason } => write!(f, "language pack {pack}: {reason}"),
            Self::Io { path, source } => write!(f, "{}: {source}", path.display()),
            Self::Serialize(message) => write!(f, "serialization failed: {message}"),
            Self::Storage(message) => write!(f, "storage failed: {message}"),
            Self::Incomplete => write!(f, "the analysis is incomplete: it lost input"),
            Self::RootSetMismatch { stored, requested } => write!(
                f,
                "the metalayer database holds the root set {stored}, not {requested}"
            ),
        }
    }
}

impl std::error::Error for ScippyError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}
