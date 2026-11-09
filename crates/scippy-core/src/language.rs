//! The contract between the core and a language pack.
//!
//! Gate 1 needs only file routing and the SCIP language name.
//! Later gates add the grammar, the binding rules, and the extraction spec.

use std::path::Path;

pub trait LanguagePack: Send + Sync {
    /// Short stable identifier, for example `python`.
    fn id(&self) -> &'static str;
    /// The SCIP language name written to `Document.language`, for example `Python`.
    fn scip_language(&self) -> &'static str;
    /// Return true when this pack owns the file.
    fn accepts(&self, path: &Path) -> bool;
}
