//! Source facts. The core runs the query of a language pack on a parse tree
//! and normalizes the captures to anchors, the join keys of the reconciler.
//!
//! The facts are ephemeral. They are not a syntax tree model.

use std::collections::BTreeMap;

use sha2::{Digest, Sha256};
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
    /// The digests of the declaration. It is `None` when the declaration has no `body` field.
    pub digests: Option<BodyDigests>,
}

/// The digests of one declaration with a body. They ignore whitespace and comments.
///
/// The tokens of the declaration, in document order, split in two parts. The body part holds
/// the tokens inside the `body` field. The signature part holds all other tokens, except the
/// name. A token is the kind and the text of a leaf node. The extent of the declaration sets
/// the tokens. A decorator of a Python function or class is not inside the extent, so it
/// is in neither digest.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct BodyDigests {
    /// The first 128 bits of the SHA-256 of the signature tokens, as lowercase hex.
    pub sig: String,
    /// The first 128 bits of the SHA-256 of the body tokens, as lowercase hex.
    pub body: String,
    /// The number of named nodes in the body, comments excluded.
    pub body_tokens: u32,
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
    /// Spans of the names of declarations that carry a type annotation, such as the `x` in
    /// `x: int = 0` or in `self.x: int = 0`.
    pub annotated: Vec<(u32, u32)>,
    /// Spans of the members of the own object of a method: the `x` in `self.x`, in `cls.x`, and in
    /// `super().x`. `self` and `cls` are the first parameter of the method that holds the
    /// reference. Any other object, such as the `Request` in `Request.x`, is not in this list.
    pub own_members: Vec<(u32, u32)>,
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
    annotated: Option<u32>,
    own_member: Option<u32>,
    own_object: Option<u32>,
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
        let mut annotated = None;
        let mut own_member = None;
        let mut own_object = None;
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
                "annotated" => annotated = Some(index),
                "own.member" => own_member = Some(index),
                "own.object" => own_object = Some(index),
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
            annotated,
            own_member,
            own_object,
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
        let mut annotated_spans = Vec::new();
        let mut own_members = Vec::new();
        let mut scopes = Vec::new();
        let mut aliases = Vec::new();
        let mut hidden_spans = Vec::new();
        let mut module = None;
        // The body of each declaration that has one, keyed by the name span and the extent.
        let mut bodies: BTreeMap<(Span, Span), Span> = BTreeMap::new();
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
                    digests: None,
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
            if let Some(annotated) = self.annotated.and_then(node) {
                annotated_spans.push((annotated.start_byte() as u32, annotated.end_byte() as u32));
            }
            if let (Some(attribute), Some(object)) = (
                self.own_member.and_then(node),
                self.own_object.and_then(node),
            ) {
                if is_own_object(object, source) {
                    own_members.push((attribute.start_byte() as u32, attribute.end_byte() as u32));
                }
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
                    digests: None,
                });
                if let Some(body) = whole.child_by_field_name("body") {
                    bodies.insert(
                        (span, (whole.start_byte() as u32, whole.end_byte() as u32)),
                        (body.start_byte() as u32, body.end_byte() as u32),
                    );
                }
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
        let entries: Vec<DigestEntry> = facts
            .declarations
            .iter()
            .enumerate()
            .filter_map(|(index, d)| {
                let name = (d.anchor.start_byte, d.anchor.end_byte);
                let body = *bodies.get(&(name, d.extent))?;
                Some(DigestEntry {
                    index,
                    extent: d.extent,
                    name,
                    body,
                })
            })
            .collect();
        for (index, digests) in digest_declarations(tree, source, entries) {
            facts.declarations[index].digests = Some(digests);
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
        annotated_spans.sort_unstable();
        annotated_spans.dedup();
        facts.annotated = annotated_spans;
        own_members.sort_unstable();
        own_members.dedup();
        facts.own_members = own_members;
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

/// A byte span: start and end.
type Span = (u32, u32);

/// A declaration that has a body, with the spans that split its tokens.
struct DigestEntry {
    /// The index of the declaration in `Facts::declarations`.
    index: usize,
    extent: (u32, u32),
    name: (u32, u32),
    body: (u32, u32),
}

/// The running hashes of one declaration.
struct DigestState {
    sig: Sha256,
    body: Sha256,
    body_tokens: u32,
}

fn hex128(hash: Sha256) -> String {
    use std::fmt::Write;
    hash.finalize()[..16]
        .iter()
        .fold(String::with_capacity(32), |mut out, b| {
            let _ = write!(out, "{b:02x}");
            out
        })
}

/// Compute the digests of all declarations with a body in one walk of the tree.
///
/// The walk visits each node once, in document order. A token goes to the hash of every
/// declaration whose extent holds it. Nested declarations need no second walk. Comments and
/// zero-width nodes add nothing, so whitespace and comments never change a digest.
fn digest_declarations(
    tree: &Tree,
    source: &str,
    mut entries: Vec<DigestEntry>,
) -> Vec<(usize, BodyDigests)> {
    if entries.is_empty() {
        return Vec::new();
    }
    entries.sort_by_key(|e| (e.extent.0, std::cmp::Reverse(e.extent.1)));
    let mut states: Vec<DigestState> = entries
        .iter()
        .map(|_| DigestState {
            sig: Sha256::new(),
            body: Sha256::new(),
            body_tokens: 0,
        })
        .collect();
    let bytes = source.as_bytes();
    let mut token = Vec::new();
    // The entries that hold the current node, from the outermost to the innermost.
    let mut active: Vec<usize> = Vec::new();
    let mut next = 0;
    let mut cursor = tree.walk();
    'walk: loop {
        let node = cursor.node();
        let (start, end) = (node.start_byte() as u32, node.end_byte() as u32);
        while active.last().is_some_and(|&i| entries[i].extent.1 <= start) {
            active.pop();
        }
        while next < entries.len() && entries[next].extent.0 <= start {
            if entries[next].extent.1 > start {
                active.push(next);
            }
            next += 1;
        }
        let outside = active.is_empty() && entries.get(next).is_none_or(|e| end <= e.extent.0);
        let mut descend = !outside && !node.kind().contains("comment");
        if descend && !active.is_empty() {
            if node.is_named() {
                for &i in &active {
                    let body = entries[i].body;
                    if body.0 <= start && end <= body.1 {
                        states[i].body_tokens += 1;
                    }
                }
            }
            if node.child_count() == 0 {
                descend = false;
                if end > start {
                    token.clear();
                    token.extend_from_slice(node.kind().as_bytes());
                    token.push(0);
                    token.extend_from_slice(&(end - start).to_le_bytes());
                    token.extend_from_slice(&bytes[start as usize..end as usize]);
                    for &i in &active {
                        let e = &entries[i];
                        if e.name.0 <= start && end <= e.name.1 {
                            continue;
                        }
                        let state = &mut states[i];
                        if e.body.0 <= start && end <= e.body.1 {
                            state.body.update(&token);
                        } else {
                            state.sig.update(&token);
                        }
                    }
                }
            }
        }
        if descend && cursor.goto_first_child() {
            continue;
        }
        loop {
            if cursor.goto_next_sibling() {
                continue 'walk;
            }
            if !cursor.goto_parent() {
                break 'walk;
            }
        }
    }
    entries
        .iter()
        .zip(states)
        .map(|(e, s)| {
            (
                e.index,
                BodyDigests {
                    sig: hex128(s.sig),
                    body: hex128(s.body),
                    body_tokens: s.body_tokens,
                },
            )
        })
        .collect()
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

/// True when `object` is the own object of a method: a call of `super()`, or a name that is the
/// first parameter of a method (`self`, `cls`) around the node. Python names only: other
/// languages have no capture of this kind.
fn is_own_object(object: tree_sitter::Node, source: &str) -> bool {
    let text = |n: tree_sitter::Node| source.get(n.start_byte()..n.end_byte()).unwrap_or("");
    match object.kind() {
        "call" => object
            .child_by_field_name("function")
            .is_some_and(|f| f.kind() == "identifier" && text(f) == "super"),
        "identifier" => {
            let name = text(object);
            let mut node = object.parent();
            while let Some(n) = node {
                if n.kind() == "function_definition"
                    && is_method_node(n)
                    && first_parameter(n, source).is_some_and(|p| p == name)
                {
                    return true;
                }
                node = n.parent();
            }
            false
        }
        _ => false,
    }
}

/// True when the function sits directly in a class body, possibly under decorators.
fn is_method_node(function: tree_sitter::Node) -> bool {
    let mut parent = function.parent();
    if parent.is_some_and(|p| p.kind() == "decorated_definition") {
        parent = parent.and_then(|p| p.parent());
    }
    parent
        .filter(|p| p.kind() == "block")
        .and_then(|b| b.parent())
        .is_some_and(|c| c.kind() == "class_definition")
}

/// The name of the first parameter of a function.
fn first_parameter<'a>(function: tree_sitter::Node, source: &'a str) -> Option<&'a str> {
    let parameters = function.child_by_field_name("parameters")?;
    let first = parameters.named_child(0)?;
    let name = match first.kind() {
        "identifier" => first,
        "typed_parameter" | "default_parameter" | "typed_default_parameter" => {
            first.named_child(0).filter(|n| n.kind() == "identifier")?
        }
        _ => return None,
    };
    source.get(name.start_byte()..name.end_byte())
}
