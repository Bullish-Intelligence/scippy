//! Source facts. The core runs the query of a language pack on a parse tree
//! and normalizes the captures to anchors, the join keys of the reconciler.
//!
//! The facts are ephemeral. They are not a syntax tree model.

use std::collections::BTreeMap;

use streaming_iterator::StreamingIterator;
use tree_sitter::{Parser, Query, QueryCursor, Tree};

use crate::error::ScippyError;
use crate::language::{Analysis, DeclKind};
use crate::project::ProjectRelativePath;

/// A byte span in one document. It is a join key, not a semantic record.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct SourceAnchor {
    pub path: ProjectRelativePath,
    pub start_byte: u32,
    pub end_byte: u32,
}

/// A declaration that the pack query found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DeclarationFact {
    /// The span of the declared identifier.
    pub anchor: SourceAnchor,
    pub name: String,
    pub kind: DeclKind,
    /// The span of the whole declaration. A container holds the declarations inside it.
    pub extent: (u32, u32),
    /// Other spans that the binding rules use as the definition site of this declaration.
    /// A binding path that ends at such a span ends at the declaration.
    pub extra_anchors: Vec<(u32, u32)>,
}

/// The facts of one document.
#[derive(Debug, Default)]
pub(crate) struct Facts {
    pub declarations: Vec<DeclarationFact>,
    /// The module of the document. Its anchor is an empty span at the start of the module node.
    pub module: Option<DeclarationFact>,
    /// Spans of identifiers that name an imported symbol.
    pub imports: Vec<(u32, u32)>,
    /// Spans of identifiers that name a member of a value, such as the `x` in `a.x`.
    pub members: Vec<(u32, u32)>,
    /// Spans of identifiers that name a parameter of a callee, such as the `x` in `f(x=1)`.
    pub keywords: Vec<(u32, u32)>,
    /// Spans of identifiers that name a supertype in the header of a class.
    pub supertypes: Vec<(u32, u32)>,
    /// Extents of the nodes that hold declarations and have no name, such as arrow functions.
    pub scopes: Vec<(u32, u32)>,
    /// Pairs of a binding name and the span of the reference that gives the name its target.
    /// The name of an import is an example: `ns` in `import * as ns from "m"` names the module.
    pub aliases: Vec<((u32, u32), (u32, u32))>,
    /// Spans whose references get no occurrence in the output.
    pub hidden: Vec<(u32, u32)>,
    /// Spans of the declarations that are second names of the module. A native pack gives them.
    pub module_aliases: Vec<(u32, u32)>,
    /// Spans of the references that read the content of their target. A native pack gives them.
    pub reads: Vec<(u32, u32)>,
    /// Declarations that two captures claimed with different kinds. The extractor drops them.
    pub conflicts: Vec<(u32, u32)>,
}

/// A syntax error. The span covers the first error node.
#[derive(Debug)]
pub(crate) struct SyntaxError {
    pub start_byte: u32,
    pub end_byte: u32,
}

/// A compiled query and a grammar. One extractor serves all files of a pack.
pub(crate) struct Extractor {
    language: tree_sitter::Language,
    query: Query,
    name: u32,
    import: Option<u32>,
    member: Option<u32>,
    keyword: Option<u32>,
    module: Option<u32>,
    supertype: Option<u32>,
    scope: Option<u32>,
    anchor: Option<u32>,
    alias_name: Option<u32>,
    alias_target: Option<u32>,
    hidden: Option<u32>,
    definitions: Vec<(u32, DeclKind)>,
}

impl Extractor {
    /// Compile the query of the pack for one grammar.
    pub fn new(
        pack: &'static str,
        analysis: &dyn Analysis,
        language: tree_sitter::Language,
        dialect_query: &str,
    ) -> Result<Self, ScippyError> {
        let bad = |reason: String| ScippyError::Pack { pack, reason };
        let text = format!("{}\n{dialect_query}", analysis.extraction_spec().query);
        let query = Query::new(&language, &text)
            .map_err(|e| bad(format!("the extraction query does not compile: {e}")))?;
        let mut name = None;
        let mut import = None;
        let mut member = None;
        let mut keyword = None;
        let mut module = None;
        let mut supertype = None;
        let mut scope = None;
        let mut anchor = None;
        let mut alias_name = None;
        let mut alias_target = None;
        let mut hidden = None;
        let mut definitions = Vec::new();
        for (index, capture) in query.capture_names().iter().enumerate() {
            let index = index as u32;
            match *capture {
                "name" => name = Some(index),
                "import" => import = Some(index),
                "member" => member = Some(index),
                "keyword" => keyword = Some(index),
                "module" => module = Some(index),
                "supertype" => supertype = Some(index),
                "scope" => scope = Some(index),
                "anchor" => anchor = Some(index),
                "alias.name" => alias_name = Some(index),
                "alias.target" => alias_target = Some(index),
                "hidden" => hidden = Some(index),
                other => {
                    if let Some(kind) = other.strip_prefix("definition.") {
                        let kind = DeclKind::from_capture(kind)
                            .ok_or_else(|| bad(format!("unknown declaration kind `{other}`")))?;
                        definitions.push((index, kind));
                    }
                }
            }
        }
        let name = name.ok_or_else(|| bad("the extraction query has no `@name` capture".into()))?;
        Ok(Self {
            language,
            query,
            name,
            import,
            member,
            keyword,
            module,
            supertype,
            scope,
            anchor,
            alias_name,
            alias_target,
            hidden,
            definitions,
        })
    }

    /// Parse the source. Return the tree, or the first syntax error.
    pub fn parse(&self, source: &str) -> Result<Tree, SyntaxError> {
        let mut parser = Parser::new();
        parser
            .set_language(&self.language)
            .expect("the grammar loaded before");
        let Some(tree) = parser.parse(source, None) else {
            return Err(SyntaxError {
                start_byte: 0,
                end_byte: 0,
            });
        };
        match first_error(&tree) {
            Some(error) => Err(error),
            None => Ok(tree),
        }
    }

    /// Run the query on a tree without syntax errors.
    pub fn facts(&self, path: &ProjectRelativePath, source: &str, tree: &Tree) -> Facts {
        let mut by_anchor: BTreeMap<(u32, u32), Vec<DeclarationFact>> = BTreeMap::new();
        let mut imports = Vec::new();
        let mut members = Vec::new();
        let mut keywords = Vec::new();
        let mut supertypes = Vec::new();
        let mut scopes = Vec::new();
        let mut aliases = Vec::new();
        let mut hidden_spans = Vec::new();
        let mut module = None;
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.query, tree.root_node(), source.as_bytes());
        while let Some(m) = matches.next() {
            let node = |index: u32| m.captures.iter().find(|c| c.index == index).map(|c| c.node);
            if let Some(import) = self.import.and_then(node) {
                imports.push((import.start_byte() as u32, import.end_byte() as u32));
            }
            if let Some(whole) = self.module.and_then(node) {
                let start = whole.start_byte() as u32;
                module = Some(DeclarationFact {
                    anchor: SourceAnchor {
                        path: path.clone(),
                        start_byte: start,
                        end_byte: start,
                    },
                    name: String::new(),
                    kind: DeclKind::Module,
                    extent: (start, whole.end_byte() as u32),
                    extra_anchors: Vec::new(),
                });
            }
            if let (Some(name), Some(target)) = (
                self.alias_name.and_then(node),
                self.alias_target.and_then(node),
            ) {
                aliases.push((
                    (name.start_byte() as u32, name.end_byte() as u32),
                    (target.start_byte() as u32, target.end_byte() as u32),
                ));
            }
            if let Some(hidden) = self.hidden.and_then(node) {
                hidden_spans.push((hidden.start_byte() as u32, hidden.end_byte() as u32));
            }
            if let Some(scope) = self.scope.and_then(node) {
                scopes.push((scope.start_byte() as u32, scope.end_byte() as u32));
            }
            if let Some(supertype) = self.supertype.and_then(node) {
                supertypes.push((supertype.start_byte() as u32, supertype.end_byte() as u32));
            }
            if let Some(member) = self.member.and_then(node) {
                members.push((member.start_byte() as u32, member.end_byte() as u32));
            }
            if let Some(keyword) = self.keyword.and_then(node) {
                keywords.push((keyword.start_byte() as u32, keyword.end_byte() as u32));
            }
            let definition = self
                .definitions
                .iter()
                .find_map(|&(index, kind)| node(index).map(|n| (kind, n)));
            if let (Some((kind, whole)), Some(name)) = (definition, node(self.name)) {
                let span = (name.start_byte() as u32, name.end_byte() as u32);
                if span.0 == span.1 {
                    continue;
                }
                let text = source[name.start_byte()..name.end_byte()].to_string();
                by_anchor.entry(span).or_default().push(DeclarationFact {
                    anchor: SourceAnchor {
                        path: path.clone(),
                        start_byte: span.0,
                        end_byte: span.1,
                    },
                    name: text,
                    kind,
                    extent: (whole.start_byte() as u32, whole.end_byte() as u32),
                    extra_anchors: self
                        .anchor
                        .and_then(node)
                        .map(|n| (n.start_byte() as u32, n.end_byte() as u32))
                        .into_iter()
                        .collect(),
                });
            }
        }

        let mut facts = Facts {
            module,
            ..Facts::default()
        };
        for (span, mut found) in by_anchor {
            found.sort_by_key(|f| (f.kind, f.extent, f.extra_anchors.clone()));
            found.dedup();
            let first = &found[0];
            // Equivalent captures merge. Captures with different kinds at one anchor conflict.
            if found.iter().any(|f| f.kind != first.kind) {
                facts.conflicts.push(span);
            } else {
                // Equivalent captures can add anchors. The declaration keeps all of them.
                let mut anchors: Vec<(u32, u32)> =
                    found.iter().flat_map(|f| f.extra_anchors.clone()).collect();
                anchors.sort_unstable();
                anchors.dedup();
                let mut first = found.swap_remove(0);
                first.extra_anchors = anchors;
                facts.declarations.push(first);
            }
        }
        imports.sort_unstable();
        imports.dedup();
        facts.imports = imports;
        members.sort_unstable();
        members.dedup();
        facts.members = members;
        keywords.sort_unstable();
        keywords.dedup();
        facts.keywords = keywords;
        supertypes.sort_unstable();
        supertypes.dedup();
        facts.supertypes = supertypes;
        scopes.sort_unstable();
        scopes.dedup();
        facts.scopes = scopes;
        aliases.sort_unstable();
        aliases.dedup();
        facts.aliases = aliases;
        hidden_spans.sort_unstable();
        hidden_spans.dedup();
        facts.hidden = hidden_spans;
        facts
    }
}

/// Find the first error or missing node in document order, without recursion.
fn first_error(tree: &Tree) -> Option<SyntaxError> {
    if !tree.root_node().has_error() {
        return None;
    }
    let mut cursor = tree.walk();
    loop {
        let node = cursor.node();
        if (node.is_error() || node.is_missing()) && node.start_byte() <= node.end_byte() {
            return Some(SyntaxError {
                start_byte: node.start_byte() as u32,
                end_byte: node.end_byte() as u32,
            });
        }
        // Enter only subtrees that hold an error.
        if node.has_error() && cursor.goto_first_child() {
            continue;
        }
        while !cursor.goto_next_sibling() {
            if !cursor.goto_parent() {
                return Some(SyntaxError {
                    start_byte: 0,
                    end_byte: 0,
                });
            }
        }
    }
}
