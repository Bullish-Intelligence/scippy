//! Symbol identities: give each declaration a symbol, and declare external assets.

use super::{Declared, Doc, SCHEME};
use crate::IndexRequest;
use crate::emit::SymbolInfo;
use crate::error::ScippyError;
use crate::extract::{DeclarationFact, Extractor, SourceAnchor};
use crate::identity::{Descriptor, IdentityBuilder, Package, SymbolId};
use crate::language::{Analysis, DeclKind, ExternalAsset, ProjectModel, Reach, Scope};
use crate::project::ProjectRelativePath;
use crate::reconcile::DeclarationIndex;
use crate::report::{Diagnostic, IndexReport, Severity, Stage};
use crate::snapshot::DeclRecord;
use std::collections::{BTreeMap, HashMap};

/// The scope that a node without a name gives to the declarations inside it.
/// The name is `@` and the byte offset of the start of the node. No identifier can have that
/// name, and two nodes never share it.
pub(super) fn anonymous_scope(extent: (u32, u32)) -> Scope {
    Scope {
        kind: DeclKind::Function,
        name: format!("@{}", extent.0),
    }
}

/// The facts of the nodes that hold declarations and have no name. They are containers only.
pub(super) fn anonymous_facts(doc: &Doc) -> Vec<DeclarationFact> {
    doc.facts
        .scopes
        .iter()
        .map(|&extent| DeclarationFact {
            anchor: SourceAnchor {
                path: doc.path.clone(),
                start_byte: extent.0,
                end_byte: extent.0,
            },
            name: anonymous_scope(extent).name,
            kind: DeclKind::Function,
            extent,
            extra_anchors: Vec::new(),
            digests: None,
        })
        .collect()
}

/// The indexes of the declarations that can hold other declarations.
pub(super) fn container_indexes(facts: &[DeclarationFact]) -> Vec<usize> {
    (0..facts.len())
        .filter(|&i| facts[i].kind.is_container())
        .collect()
}

/// The innermost container that holds declaration `i`, when there is one.
pub(super) fn innermost_container(
    facts: &[DeclarationFact],
    containers: &[usize],
    i: usize,
) -> Option<usize> {
    let fact = &facts[i];
    containers
        .iter()
        .copied()
        .filter(|&j| {
            j != i && facts[j].extent.0 <= fact.extent.0 && fact.extent.1 <= facts[j].extent.1
        })
        .min_by_key(|&j| facts[j].extent.1 - facts[j].extent.0)
}

/// The enclosing declarations of declaration `i`, from the outermost to the innermost.
pub(super) fn scope_chain(facts: &[DeclarationFact], containers: &[usize], i: usize) -> Vec<Scope> {
    let fact = &facts[i];
    let mut ancestors: Vec<&DeclarationFact> = containers
        .iter()
        .filter(|&&j| {
            j != i && facts[j].extent.0 <= fact.extent.0 && fact.extent.1 <= facts[j].extent.1
        })
        .map(|&j| &facts[j])
        .collect();
    ancestors.sort_by_key(|a| (a.extent.0, std::cmp::Reverse(a.extent.1)));
    ancestors
        .iter()
        .map(|a| Scope {
            kind: a.kind,
            name: a.name.clone(),
        })
        .collect()
}

/// The identity builder of a run. The request can replace the name and the version of the package.
pub(super) fn identity_builder(request: &IndexRequest, mut package: Package) -> IdentityBuilder {
    if let Some(name) = &request.package_name {
        package.name = name.clone();
    }
    if let Some(version) = &request.package_version {
        package.version = version.clone();
    }
    IdentityBuilder::new(SCHEME, package)
}

/// Give each declaration of a document its symbol.
pub(super) fn assign_identities(
    model: &dyn ProjectModel,
    builder: &mut IdentityBuilder,
    doc: &Doc,
) -> Result<Vec<Declared>, ScippyError> {
    // The declarations come first. The nodes without a name follow. They are containers only.
    let declared_count = doc.facts.declarations.len();
    let mut facts = doc.facts.declarations.clone();
    facts.extend(anonymous_facts(doc));
    let facts = &facts;
    let containers = container_indexes(facts);
    let mut locals: BTreeMap<Vec<Descriptor>, u32> = BTreeMap::new();
    let mut out = Vec::with_capacity(declared_count + 1);
    // The module of the document. The pack decides whether it has a symbol.
    if let (Some(fact), Some(identity)) = (&doc.facts.module, model.module_identity(&doc.path)) {
        if identity.reach == Reach::Global {
            let symbol = builder.define(&identity.descriptors, fact.anchor.clone())?;
            out.push(Declared {
                fact: fact.clone(),
                symbol,
                kind: identity.kind,
                scopes: Vec::new(),
                declaration: Scope {
                    kind: DeclKind::Module,
                    name: identity.display_name.clone(),
                },
                display_name: identity.display_name,
                decl: Some(DeclRecord::default()),
            });
        }
    }
    // The declarations that each container holds directly, keyed by the extent of the container.
    let holders: Vec<Option<(u32, u32)>> = (0..facts.len())
        .map(|i| innermost_container(facts, &containers, i).map(|j| facts[j].extent))
        .collect();
    let mut held: HashMap<Option<(u32, u32)>, Vec<Scope>> = HashMap::new();
    for (fact, holder) in facts.iter().zip(&holders).take(declared_count) {
        held.entry(*holder).or_default().push(Scope {
            kind: fact.kind,
            name: fact.name.clone(),
        });
    }
    // The index of the first declaration in `out`. The module can come before it.
    let first = out.len();
    let mut global = vec![false; declared_count];
    for (i, fact) in facts.iter().enumerate().take(declared_count) {
        let scopes = scope_chain(facts, &containers, i);
        let declaration = Scope {
            kind: fact.kind,
            name: fact.name.clone(),
        };
        let siblings: Vec<Scope> = held[&holders[i]]
            .iter()
            .filter(|s| **s != declaration)
            .cloned()
            .collect();
        let identity = model.identity(&doc.path, &scopes, &declaration, &siblings);
        let symbol = match identity.reach {
            Reach::Global => builder.define(&identity.descriptors, fact.anchor.clone())?,
            Reach::Document => {
                let next = locals.len() as u32;
                SymbolId::local(*locals.entry(identity.descriptors).or_insert(next))
            }
        };
        global[i] = identity.reach == Reach::Global;
        let carries = !matches!(fact.kind, DeclKind::TypeParameter | DeclKind::Parameter);
        let decl = (global[i] && carries).then(|| DeclRecord {
            container: scopes.iter().map(|s| s.name.clone()).collect(),
            sig_digest: fact.digests.as_ref().map(|d| d.sig.clone()),
            body_digest: fact.digests.as_ref().map(|d| d.body.clone()),
            body_tokens: fact.digests.as_ref().map(|d| d.body_tokens),
            ..DeclRecord::default()
        });
        out.push(Declared {
            fact: fact.clone(),
            symbol,
            kind: identity.kind,
            display_name: identity.display_name,
            scopes,
            declaration,
            decl,
        });
    }
    // A parameter carries its facts when a global function or method holds it. The rule reads
    // the holder, not the reach of the parameter. The parent symbol builds the identity key.
    for i in 0..declared_count {
        if facts[i].kind != DeclKind::Parameter {
            continue;
        }
        let Some(holder) = innermost_container(facts, &containers, i) else {
            continue;
        };
        if holder >= declared_count || facts[holder].kind != DeclKind::Function || !global[holder] {
            continue;
        }
        let parent = out[first + holder].symbol.as_str().to_owned();
        out[first + i].decl = Some(DeclRecord {
            container: out[first + i]
                .scopes
                .iter()
                .map(|s| s.name.clone())
                .collect(),
            parent: Some(parent),
            ..DeclRecord::default()
        });
    }
    Ok(out)
}

/// Parse the external assets of the pack and give their declarations identities.
///
/// The assets are the builtins and the other assets that the pack lists, such as stub modules.
/// The declarations join to references by anchor, like project declarations.
/// The result maps each external symbol to its metadata.
pub(super) fn declare_external(
    extractor: &Extractor,
    analysis: &dyn Analysis,
    model: &dyn ProjectModel,
    report: &mut IndexReport,
    index: &mut DeclarationIndex,
) -> BTreeMap<SymbolId, SymbolInfo> {
    let mut symbols = BTreeMap::new();
    for asset in analysis.binding_rules().external_assets() {
        declare_asset(extractor, &asset, model, report, index, &mut symbols);
    }
    symbols
}

/// Give the declarations of one external asset their identities.
pub(super) fn declare_asset(
    extractor: &Extractor,
    asset: &ExternalAsset,
    model: &dyn ProjectModel,
    report: &mut IndexReport,
    index: &mut DeclarationIndex,
    symbols: &mut BTreeMap<SymbolId, SymbolInfo>,
) {
    if asset.source.trim().is_empty() {
        return;
    }
    let warn = |code, message: String| Diagnostic {
        severity: Severity::Warning,
        stage: Stage::Identity,
        file: None,
        range: None,
        code,
        message,
    };
    let Ok(path) = ProjectRelativePath::new(&asset.name) else {
        return;
    };
    let tree = match extractor.parse(asset.source) {
        Ok(tree) => tree,
        Err(_) => {
            report.diagnostics.push(warn(
                "external-asset-syntax-error",
                format!(
                    "the asset {} has a syntax error. Its names stay unresolved",
                    asset.path
                ),
            ));
            return;
        }
    };
    let facts = extractor.facts(&path, asset.source, &tree);
    let declarations = &facts.declarations;
    let containers = container_indexes(declarations);
    // The module of the asset is a declaration too. Its name is the stem of the file name.
    let stem = asset.name.rsplit('/').next().unwrap_or(&asset.name);
    let module = facts.module.iter().map(|fact| {
        let name = stem.rsplit_once('.').map_or(stem, |(stem, _)| stem);
        let declaration = Scope {
            kind: DeclKind::Module,
            name: name.to_string(),
        };
        (fact, Vec::new(), declaration)
    });
    let members = declarations.iter().enumerate().map(|(i, fact)| {
        let declaration = Scope {
            kind: fact.kind,
            name: fact.name.clone(),
        };
        (fact, scope_chain(declarations, &containers, i), declaration)
    });
    for (fact, scopes, declaration) in module.chain(members) {
        let Some(identity) = model.external_identity(&asset.name, &scopes, &declaration) else {
            continue;
        };
        match IdentityBuilder::new(SCHEME, identity.package).symbol(&identity.descriptors) {
            Ok(symbol) => {
                index.insert_external_scoped(
                    fact.anchor.clone(),
                    symbol.clone(),
                    scopes.clone(),
                    declaration.clone(),
                );
                symbols.entry(symbol.clone()).or_insert(SymbolInfo {
                    symbol,
                    kind: identity.kind,
                    display_name: identity.display_name,
                    implements: Vec::new(),
                    references: Vec::new(),
                    decl: None,
                });
            }
            Err(error) => report
                .diagnostics
                .push(warn("external-symbol-invalid", error.to_string())),
        }
    }
}
