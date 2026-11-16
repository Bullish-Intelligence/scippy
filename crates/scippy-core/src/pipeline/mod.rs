//! The indexing pipeline: discover, parse, extract, bind, reconcile, identify, emit.
//!
//! - `run`: the orchestration of one run.
//! - `stack`, `shard`, `load`, `memory`: the stack-graphs pack path.
//! - `native`: the native pack path.
//! - `identities`, `finish`: the stages that both paths share.
//! - `inputs`: the manifests and the support files.

mod finish;
mod identities;
mod inputs;
mod load;
mod memory;
mod native;
mod run;
mod shard;
mod stack;

pub(crate) use run::run;

use crate::IndexRequest;
use crate::emit::{Document, SymbolInfo};
use crate::error::ScippyError;
use crate::extract::{DeclarationFact, Facts};
use crate::identity::SymbolId;
use crate::language::{ExtraFile, LanguagePack, Scope, WalkRules};
use crate::project::{self, FoundFile, ProjectRelativePath, Router};
use crate::report::IndexReport;
use crate::snapshot::{Capture, DeclRecord};
use crate::source::SourceStore;
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

/// The SCIP scheme of every symbol that Scippy writes.
/// It is Scippy-owned. Scippy does not claim the identifiers of another indexer.
pub(crate) const SCHEME: &str = "scippy";

struct PackRoute<'a> {
    pack: &'a dyn LanguagePack,
    rules: WalkRules,
}

impl Router for PackRoute<'_> {
    fn id(&self) -> &'static str {
        self.pack.id()
    }
    fn accepts(&self, path: &Path) -> bool {
        self.pack.accepts(path)
    }
    fn enter_ignored(&self, relative_dir: &str) -> bool {
        self.rules
            .enter_ignored
            .as_ref()
            .is_some_and(|enter| enter(relative_dir))
    }
    fn hidden(&self, relative: &str) -> bool {
        self.rules
            .hidden
            .as_ref()
            .is_some_and(|hidden| hidden(relative))
    }
}

/// A document that parsed and built.
struct Doc {
    path: ProjectRelativePath,
    /// The text that the source store read once. Every stage shares it.
    source: Arc<str>,
    facts: Facts,
    /// The declarations with their symbols, in anchor order.
    declared: Vec<Declared>,
}

struct Declared {
    fact: DeclarationFact,
    symbol: SymbolId,
    kind: crate::emit::SymbolKind,
    display_name: String,
    /// The enclosing declarations and the declaration itself, for the target policy of the pack.
    scopes: Vec<Scope>,
    declaration: Scope,
    /// The identity facts. Only a global declaration and a parameter of a global function carry
    /// them.
    decl: Option<DeclRecord>,
}

/// What the index of one pack shares with the run: the pack, its files, and the run outputs
/// that the pack adds to.
struct PackRun<'a> {
    pack: &'a dyn LanguagePack,
    root: &'a Path,
    /// The files that the router gave to the pack.
    files: &'a [&'a FoundFile],
    store: &'a SourceStore,
    request: &'a IndexRequest,
    report: &'a mut IndexReport,
    trace: &'a mut Vec<String>,
    capture: &'a mut Option<Capture>,
}

/// What one pack gives back: its documents and the external symbols that they reference.
struct PackIndex {
    documents: Vec<Document>,
    /// Documents of a native pack that only define symbols. See [`NativeDocument::stub`].
    stubs: Vec<Document>,
    external_symbols: BTreeMap<SymbolId, SymbolInfo>,
}

/// List the files that the walk of [`run`] accepts, in walk order. It reads no file content and
/// writes nothing. `request.output` and `request.limits` act as in a run. Only the packs that
/// are enabled for `request.root` take part.
pub fn discover_files(
    request: &IndexRequest,
    languages: &[&dyn LanguagePack],
) -> Result<Vec<ProjectRelativePath>, ScippyError> {
    let languages: Vec<&dyn LanguagePack> = languages
        .iter()
        .copied()
        .filter(|p| p.enabled(&request.root))
        .collect();
    let routes: Vec<PackRoute> = languages
        .iter()
        .map(|p| PackRoute {
            pack: *p,
            rules: p.walk_rules(&request.root),
        })
        .collect();
    let routers: Vec<&dyn Router> = routes.iter().map(|r| r as &dyn Router).collect();
    let extra: Vec<ExtraFile> = languages
        .iter()
        .flat_map(|p| p.extra_files(&request.root).files)
        .collect();
    let found = project::discover(
        &request.root,
        &request.output,
        &routers,
        request.limits,
        &extra,
    )?;
    Ok(found.files.into_iter().map(|f| f.path).collect())
}
