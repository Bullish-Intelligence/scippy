//! Source facts. A later stage fills them from the parse trees of a language pack.

use crate::project::ProjectRelativePath;

/// A byte span in one document. It is a join key, not a semantic record.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct SourceAnchor {
    pub path: ProjectRelativePath,
    pub start_byte: u32,
    pub end_byte: u32,
}
