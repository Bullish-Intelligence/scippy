//! The rebinding engine: give each declaration of a new run an old entity id or a new id.
//!
//! The design is in `METALAYER_IDENTITY.md`, sections 5 and 6. The engine runs in one
//! transaction of the identity store. It never reads or writes the facts store.
//!
//! The engine is generic over the family of the entities. A family supplies its items (the
//! declarations of the new run), and its tier list. The code family has tiers T1, T1h, T2f, T2,
//! T3, T4, T5, then the parameters. The note family (section 7 of the design) binds the notes
//! first: T1, T1h, T2 for a note whose key changed with its path, and M2. Then it binds the
//! headings and blocks of the notes: the duplicate group rule, T1, T1h, M3, M4, and M5.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fmt::Write;

use rusqlite::{OptionalExtension, Transaction, TransactionBehavior, params};
use scippy_core::{AnalysisSnapshot, ScippyError};

use crate::identity::{
    EntityKind, Family, IdSource, Identity, Locator, Outcome, OutcomeRow, Tier, join_container,
    push_locator_in,
};
use crate::schema::storage;

/// The smallest `body_tokens` that lets a body digest bind (tiers T4 and T5). A body below this
/// size is too short to tell two declarations apart. `id-eval` calibrates the value.
pub const MIN_BODY_TOKENS: i64 = 10;

/// The environment variable that overrides [`MIN_BODY_TOKENS`] for one run. Evaluation sweeps
/// use it. It is not a stable interface: a later version may remove it.
pub const MIN_BODY_TOKENS_ENV: &str = "SCIPPY_IDENTITY_MIN_BODY_TOKENS";

/// The minimum body size of this run: the variable [`MIN_BODY_TOKENS_ENV`] when it holds a
/// whole number, else [`MIN_BODY_TOKENS`].
pub fn min_body_tokens() -> i64 {
    std::env::var(MIN_BODY_TOKENS_ENV)
        .ok()
        .and_then(|text| text.trim().parse().ok())
        .unwrap_or(MIN_BODY_TOKENS)
}

/// An unannotated orphan goes after this many rebindings.
pub const ORPHAN_GC_PUBLISHES: usize = 100;

/// The `meta` key of the digests of the most recent rebindings, one per line.
const META_RECENT_DIGESTS: &str = "recent_digests";

/// The `meta` key of the digest of the items of the last rebinding. Publish skips the
/// rebinding only when the input digest and this digest both match.
pub(crate) const META_BOUND_ITEMS_DIGEST: &str = "bound_items_digest";

/// The `meta` key of the items digest of a family.
pub(crate) fn items_meta_key(family: Family) -> &'static str {
    match family {
        Family::Code => META_BOUND_ITEMS_DIGEST,
        Family::Note => "bound_items_digest_note",
    }
}

/// The separator of the parts of a compound key in memory.
const SEP: char = '\u{1f}';

// ---- Keys ----

/// The identity key of a code declaration: the SCIP symbol with the package version removed.
///
/// The version field becomes `.`, the SCIP text for an empty field. A local symbol and a text
/// that is not a symbol give `None`.
pub fn symbol_key(symbol: &str) -> Option<String> {
    if symbol.starts_with("local ") {
        return None;
    }
    let bytes = symbol.as_bytes();
    let mut at = 0;
    let mut version = (0, 0);
    for field in 0..4 {
        let start = at;
        while at < bytes.len() {
            if bytes[at] == b' ' {
                // A double space is a space inside a field.
                if bytes.get(at + 1) == Some(&b' ') {
                    at += 2;
                    continue;
                }
                break;
            }
            at += 1;
        }
        if field == 3 {
            version = (start, at);
        } else if at >= bytes.len() {
            return None;
        }
        at += 1;
    }
    Some(format!("{}.{}", &symbol[..version.0], &symbol[version.1..]))
}

/// The identity key of a child: the entity id of the parent and the name of the child.
pub fn child_key(parent_id: &str, name: &str) -> String {
    format!("{parent_id}/{name}")
}

/// The entity kind of a SCIP kind name.
pub fn entity_kind(scip_kind: &str) -> EntityKind {
    match scip_kind {
        "Module" => EntityKind::Module,
        "Class" | "Interface" | "Enum" | "TypeAlias" | "Namespace" => EntityKind::Class,
        "Function" => EntityKind::Function,
        "Method" => EntityKind::Method,
        "Parameter" => EntityKind::Parameter,
        _ => EntityKind::Variable,
    }
}

// ---- Input and output ----

/// One declaration of the new run.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Item {
    /// The identity key. A child has no key of its own: the engine builds it from the entity id
    /// of the parent and `child_name`. The field of a child is ignored.
    pub key: String,
    pub kind: EntityKind,
    pub symbol: Option<String>,
    pub path: String,
    pub name: String,
    /// The display names of the containers, joined by `CONTAINER_SEPARATOR`.
    pub container: String,
    pub sig_digest: Option<String>,
    pub body_digest: Option<String>,
    pub body_tokens: Option<i64>,
    /// The key of the parent item, for a child (a parameter).
    pub parent_key: Option<String>,
    /// The name of a child below its parent.
    pub child_name: Option<String>,
}

impl Item {
    fn is_child(&self) -> bool {
        self.parent_key.is_some()
    }

    fn variable(&self) -> bool {
        self.kind == EntityKind::Variable
    }
}

/// What one rebinding reads.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct RebindInput {
    /// The `input_digest` of the facts of the new run.
    pub input_digest: String,
    pub family: Family,
    pub items: Vec<Item>,
    /// The path of each file of the new run, with its `sha256` when it has one.
    pub files: BTreeMap<String, Option<String>>,
    /// The paths and hashes of the previous run, when the caller knows them for the same
    /// bound input. Tier T2f reads them to pair a renamed file. `None` leaves the probe only.
    pub old_files: Option<BTreeMap<String, String>>,
}

impl RebindInput {
    /// The input of the code family from a snapshot.
    pub fn from_snapshot(
        snapshot: &AnalysisSnapshot,
        input_digest: &str,
        old_files: Option<BTreeMap<String, String>>,
    ) -> Self {
        let mut items = Vec::new();
        let mut seen: HashSet<(Option<String>, String)> = HashSet::new();
        let mut files = BTreeMap::new();
        for lost in &snapshot.lost_files {
            files.insert(lost.path.clone(), lost.sha256.clone());
        }
        for doc in &snapshot.documents {
            files.insert(doc.path.clone(), doc.sha256.clone());
            // The note family reads the documents of the Loci pack.
            if doc.pack.as_deref() == Some(LOCI_PACK) {
                continue;
            }
            for sym in &doc.symbols {
                let Some(decl) = &sym.decl else { continue };
                if decl.note.is_some() {
                    continue;
                }
                let Some(key) = symbol_key(&sym.symbol) else {
                    continue;
                };
                let kind = entity_kind(&sym.kind);
                let (key, parent_key, child_name) = if kind == EntityKind::Parameter {
                    let Some(parent) = decl.parent.as_deref().and_then(symbol_key) else {
                        continue;
                    };
                    (String::new(), Some(parent), Some(sym.display_name.clone()))
                } else {
                    (key, None, None)
                };
                let dedupe = match &parent_key {
                    Some(parent) => (Some(parent.clone()), sym.display_name.clone()),
                    None => (None, key.clone()),
                };
                if !seen.insert(dedupe) {
                    continue;
                }
                items.push(Item {
                    key,
                    kind,
                    symbol: Some(sym.symbol.clone()),
                    path: doc.path.clone(),
                    name: sym.display_name.clone(),
                    container: join_container(&decl.container),
                    sig_digest: decl.sig_digest.clone(),
                    body_digest: decl.body_digest.clone(),
                    body_tokens: decl.body_tokens.map(i64::from),
                    parent_key,
                    child_name,
                });
            }
        }
        // A fixed order gives fixed ids when the id source counts.
        items.sort_by(|a, b| {
            (&a.path, &a.key, &a.child_name).cmp(&(&b.path, &b.key, &b.child_name))
        });
        Self {
            input_digest: input_digest.into(),
            family: Family::Code,
            items,
            files,
            old_files,
        }
    }
}

/// The id of the Loci language pack.
const LOCI_PACK: &str = "loci";

/// The identity key of a note: `id:<loci.id>`, else `path:<path>`.
pub fn note_key(loci_id: Option<&str>, path: &str) -> String {
    match loci_id {
        Some(id) => format!("id:{id}"),
        None => format!("path:{path}"),
    }
}

/// The part of the key of a heading below the key of its note: the texts of the ancestors, the
/// text, and the ordinal among equal siblings.
pub fn heading_suffix(ancestors: &[String], text: &str, ordinal: u32) -> String {
    format!("{}/{text}#{ordinal}", ancestors.join("/"))
}

/// The part of the key of a block below the key of its note.
pub fn block_suffix(block_id: &str) -> String {
    format!("^{block_id}")
}

impl RebindInput {
    /// The input of the note family from a snapshot: the notes, and their headings and blocks.
    ///
    /// A `loci.id` that two notes carry is ignored: both notes get a path key.
    pub fn notes_from_snapshot(snapshot: &AnalysisSnapshot, input_digest: &str) -> Self {
        let record = |sym: &scippy_core::AnalysisSymbol| {
            sym.decl.as_ref().and_then(|d| d.note.as_ref()).cloned()
        };
        let mut counts: HashMap<String, usize> = HashMap::new();
        for doc in &snapshot.documents {
            for sym in &doc.symbols {
                if let Some(note) = record(sym) {
                    if let (Some(id), "note") = (note.loci_id, note.kind.as_str()) {
                        *counts.entry(id).or_default() += 1;
                    }
                }
            }
        }
        let mut items = Vec::new();
        let mut files = BTreeMap::new();
        for doc in &snapshot.documents {
            let notes: Vec<_> = doc
                .symbols
                .iter()
                .filter_map(|s| record(s).map(|n| (s, n)))
                .collect();
            let Some((sym, note)) = notes.iter().find(|(_, n)| n.kind == "note") else {
                continue;
            };
            files.insert(doc.path.clone(), doc.sha256.clone());
            let loci_id = note
                .loci_id
                .as_deref()
                .filter(|id| counts.get(*id) == Some(&1));
            let key = note_key(loci_id, &doc.path);
            items.push(Item {
                key: key.clone(),
                kind: EntityKind::Note,
                symbol: Some(sym.symbol.clone()),
                path: doc.path.clone(),
                name: doc.path.rsplit('/').next().unwrap_or(&doc.path).to_owned(),
                container: String::new(),
                sig_digest: None,
                body_digest: doc.sha256.clone(),
                body_tokens: None,
                parent_key: None,
                child_name: None,
            });
            let mut seen: HashSet<String> = HashSet::new();
            for (sym, note) in &notes {
                let (kind, suffix, name, container) = match note.kind.as_str() {
                    "heading" => {
                        let (Some(text), Some(ordinal)) = (&note.text, note.ordinal) else {
                            continue;
                        };
                        (
                            EntityKind::Heading,
                            heading_suffix(&note.ancestors, text, ordinal),
                            text.clone(),
                            join_container(&note.ancestors),
                        )
                    }
                    "block" => {
                        let Some(id) = &note.block_id else { continue };
                        (
                            EntityKind::Block,
                            block_suffix(id),
                            block_suffix(id),
                            String::new(),
                        )
                    }
                    _ => continue,
                };
                // The first of two equal block ids wins, as it does in a link.
                if !seen.insert(format!("{}{SEP}{suffix}", kind.as_str())) {
                    continue;
                }
                items.push(Item {
                    key: String::new(),
                    kind,
                    symbol: Some(sym.symbol.clone()),
                    path: doc.path.clone(),
                    name,
                    container,
                    sig_digest: None,
                    body_digest: note.section_digest.clone(),
                    body_tokens: note.section_tokens.map(i64::from),
                    parent_key: Some(key.clone()),
                    child_name: Some(suffix),
                });
            }
        }
        items.sort_by(|a, b| {
            (&a.path, &a.key, &a.child_name).cmp(&(&b.path, &b.key, &b.child_name))
        });
        Self {
            input_digest: input_digest.into(),
            family: Family::Note,
            items,
            files,
            old_files: None,
        }
    }

    /// A digest of everything in the items that a rebinding reads or writes: the symbol, the
    /// path, the name, the container, and the digests. A change of the package version changes
    /// the symbol strings and so this digest, while the input digest of the facts can stay.
    pub fn items_digest(&self) -> String {
        let mut text = String::new();
        let mut field = |value: &str| {
            let _ = writeln!(text, "{}:{value}", value.len());
        };
        field(self.family.as_str());
        for item in &self.items {
            field(&item.key);
            field(item.kind.as_str());
            field(item.symbol.as_deref().unwrap_or(""));
            field(&item.path);
            field(&item.name);
            field(&item.container);
            field(item.sig_digest.as_deref().unwrap_or(""));
            field(item.body_digest.as_deref().unwrap_or(""));
            field(&item.body_tokens.map_or_else(String::new, |t| t.to_string()));
            field(item.parent_key.as_deref().unwrap_or(""));
            field(item.child_name.as_deref().unwrap_or(""));
        }
        crate::publish::sha256_hex(text.as_bytes())
    }
}

/// What one rebinding did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct RebindReport {
    /// The store was already bound to this input. Nothing ran.
    pub skipped: bool,
    /// The entities that each tier bound, by the text of the tier.
    pub bound: BTreeMap<&'static str, usize>,
    /// The ids that the run minted.
    pub minted: usize,
    /// The old entities that the run left `ambiguous`.
    pub ambiguous: usize,
    /// The old entities that the run left `orphaned`.
    pub orphaned: usize,
    /// The old entities of the pool that the run deleted (unannotated old orphans).
    pub collected: usize,
}

impl RebindReport {
    /// The number of entities that the tiers bound.
    pub fn bound_total(&self) -> usize {
        self.bound.values().sum()
    }

    /// Add the counts of the report of another family.
    pub fn merge(&mut self, other: RebindReport) {
        self.skipped &= other.skipped;
        for (tier, n) in other.bound {
            *self.bound.entry(tier).or_default() += n;
        }
        self.minted += other.minted;
        self.ambiguous += other.ambiguous;
        self.orphaned += other.orphaned;
        self.collected += other.collected;
    }
}

// ---- The pool ----

/// One old entity with the data that the tiers read.
struct Old {
    id: String,
    kind: EntityKind,
    parent_id: Option<String>,
    cur: Locator,
    /// The keys of the locators from seq 1 up, for tier T1h.
    older_keys: Vec<String>,
    outcome: Option<OutcomeRow>,
}

impl Old {
    fn is_child(&self) -> bool {
        self.parent_id.is_some()
    }
}

/// The result of one tier: the one-to-one pairs, and the candidates of each old entity that
/// has candidates but no pair.
type Found = (Vec<(usize, usize)>, Vec<(usize, Vec<usize>)>);

/// How a tier pairs old entities with items.
struct Spec<'f> {
    tier: Tier,
    /// The keys of an old entity. The empty list means: the tier skips it.
    old_keys: &'f dyn Fn(&Old) -> Vec<String>,
    /// The key of an item. `None` means: the tier skips it.
    new_key: &'f dyn Fn(&Item) -> Option<String>,
    /// A last test on a pair.
    accept: &'f dyn Fn(&Old, &Item) -> bool,
    /// Several candidates: keep those with an equal signature digest, then an equal body digest.
    narrow: bool,
}

/// The state of one rebinding: the old entities, the new items, and the pairs so far.
pub(crate) struct Pool<'a> {
    input: &'a RebindInput,
    /// The items of the input. The note family fills the keys of the children, and sets
    /// `parent_key` to the entity id of the parent.
    items: Vec<Item>,
    /// The tiers read the children, not the parents.
    children: bool,
    /// An old entity or an item that a group rule settled. Later tiers skip it.
    old_frozen: Vec<bool>,
    new_frozen: Vec<bool>,
    olds: Vec<Old>,
    /// For each old entity, the item that it bound to, and the tier.
    old_bound: Vec<Option<(usize, Tier)>>,
    /// For each item, the old entity that it bound to.
    new_bound: Vec<Option<usize>>,
    /// For each old entity, the tier that first left it candidates, and the items.
    hints: Vec<Option<(Tier, BTreeSet<usize>)>>,
    vetoes: HashMap<String, HashSet<String>>,
    min_tokens: i64,
}

/// True when both digests of the signature exist and are equal.
fn same_sig(o: &Old, i: &Item) -> bool {
    o.cur.sig_digest.is_some() && o.cur.sig_digest == i.sig_digest
}

/// True when both digests of the body exist and are equal, and the body has `min` tokens or
/// more. A short body, such as `pass`, is equal in many places.
fn same_body(min: i64, o: &Old, i: &Item) -> bool {
    o.cur.body_digest.is_some()
        && o.cur.body_digest == i.body_digest
        && i.body_tokens.is_some_and(|t| t >= min)
}

/// T3 accepts a pair in another path that has an equal container or an equal body.
fn t3_accept(min: i64, o: &Old, i: &Item) -> bool {
    o.cur.path != i.path && (o.cur.container == i.container || same_body(min, o, i))
}

impl<'a> Pool<'a> {
    fn item(&self, n: usize) -> &Item {
        &self.items[n]
    }

    fn vetoed(&self, old: &Old, item: &Item) -> bool {
        self.vetoes
            .get(&old.id)
            .is_some_and(|keys| keys.contains(&item.key))
    }

    /// Keep the items with an equal signature digest, then those with an equal body digest.
    /// A step that would keep none changes nothing.
    fn narrow(&self, mut found: Vec<usize>, sig: Option<&str>, body: Option<&str>) -> Vec<usize> {
        if found.len() > 1 {
            let same: Vec<usize> = found
                .iter()
                .copied()
                .filter(|&n| sig.is_some() && self.item(n).sig_digest.as_deref() == sig)
                .collect();
            if !same.is_empty() {
                found = same;
            }
        }
        if found.len() > 1 {
            let same: Vec<usize> = found
                .iter()
                .copied()
                .filter(|&n| body.is_some() && self.item(n).body_digest.as_deref() == body)
                .collect();
            if !same.is_empty() {
                found = same;
            }
        }
        found
    }

    fn bind(&mut self, o: usize, n: usize, tier: Tier) {
        self.old_bound[o] = Some((n, tier));
        self.new_bound[n] = Some(o);
    }

    /// Find the pairs of one tier without applying them. The result holds the one-to-one pairs
    /// and, for each old entity that has candidates but no pair, its candidates.
    fn find(&self, spec: &Spec<'_>) -> Found {
        let mut groups: HashMap<String, Vec<usize>> = HashMap::new();
        for (n, item) in self.items.iter().enumerate() {
            if item.is_child() != self.children || self.new_bound[n].is_some() || self.new_frozen[n]
            {
                continue;
            }
            if let Some(key) = (spec.new_key)(item) {
                groups.entry(key).or_default().push(n);
            }
        }
        // The candidates of each unbound old entity.
        let mut candidates: Vec<(usize, Vec<usize>)> = Vec::new();
        for (o, old) in self.olds.iter().enumerate() {
            if old.is_child() != self.children || self.old_bound[o].is_some() || self.old_frozen[o]
            {
                continue;
            }
            let mut found: Vec<usize> = Vec::new();
            for key in (spec.old_keys)(old) {
                if let Some(group) = groups.get(&key) {
                    found.extend(group.iter().copied());
                }
            }
            found.sort_unstable();
            found.dedup();
            found.retain(|&n| {
                let item = self.item(n);
                (spec.accept)(old, item) && !self.vetoed(old, item)
            });
            if found.is_empty() {
                continue;
            }
            if spec.narrow {
                found = self.narrow(
                    found,
                    old.cur.sig_digest.as_deref(),
                    old.cur.body_digest.as_deref(),
                );
            }
            candidates.push((o, found));
        }
        // The claimants of each item.
        let mut claimants: HashMap<usize, Vec<usize>> = HashMap::new();
        for (o, found) in &candidates {
            for &n in found {
                claimants.entry(n).or_default().push(*o);
            }
        }
        if spec.narrow {
            for (&n, claim) in claimants.iter_mut() {
                if claim.len() < 2 {
                    continue;
                }
                let item = self.item(n);
                let mut kept = claim.clone();
                if item.sig_digest.is_some() {
                    let same: Vec<usize> = kept
                        .iter()
                        .copied()
                        .filter(|&o| self.olds[o].cur.sig_digest == item.sig_digest)
                        .collect();
                    if !same.is_empty() {
                        kept = same;
                    }
                }
                if kept.len() > 1 && item.body_digest.is_some() {
                    let same: Vec<usize> = kept
                        .iter()
                        .copied()
                        .filter(|&o| self.olds[o].cur.body_digest == item.body_digest)
                        .collect();
                    if !same.is_empty() {
                        kept = same;
                    }
                }
                *claim = kept;
            }
        }
        let mut pairs = Vec::new();
        let mut left = Vec::new();
        for (o, found) in candidates {
            if found.len() == 1 && claimants[&found[0]] == [o] {
                pairs.push((o, found[0]));
            } else {
                left.push((o, found));
            }
        }
        (pairs, left)
    }

    /// Run one tier: bind the pairs, and note the candidates of the rest.
    fn run(&mut self, spec: &Spec<'_>) {
        let (pairs, left) = self.find(spec);
        for (o, n) in pairs {
            self.bind(o, n, spec.tier);
        }
        for (o, found) in left {
            let hint = self.hints[o].get_or_insert((spec.tier, BTreeSet::new()));
            hint.1.extend(found);
        }
    }

    /// The code tiers, in order, then the children.
    fn run_code_tiers(&mut self) {
        let min = self.min_tokens;
        let tokens = move |t: Option<i64>| t.is_some_and(|t| t >= min);
        self.run(&Spec {
            tier: Tier::T1,
            old_keys: &|o| vec![o.cur.key.clone()],
            new_key: &|i| Some(i.key.clone()),
            accept: &|_, _| true,
            narrow: false,
        });
        self.run(&Spec {
            tier: Tier::T1H,
            old_keys: &|o| o.older_keys.clone(),
            new_key: &|i| Some(i.key.clone()),
            accept: &|_, _| true,
            narrow: false,
        });

        // T2f: bind inside a pair of files.
        let pairs = self.pair_files();
        if !pairs.is_empty() {
            // A module has one name for each file, and the name follows the file.
            let place = |path: &str, container: &str, name: &str, kind: &str| {
                let name = if kind == EntityKind::Module.as_str() {
                    ""
                } else {
                    name
                };
                format!("{path}{SEP}{container}{SEP}{name}{SEP}{kind}")
            };
            self.run(&Spec {
                tier: Tier::T2F,
                old_keys: &|o| match pairs.get(&o.cur.path) {
                    Some(to) => vec![place(to, &o.cur.container, &o.cur.name, &o.cur.kind)],
                    None => Vec::new(),
                },
                new_key: &|i| Some(place(&i.path, &i.container, &i.name, i.kind.as_str())),
                accept: &|_, _| true,
                narrow: true,
            });
        }

        // T2: the same path, name, and kind. Another container needs an equal signature and body.
        let by_place = |path: &str, name: &str, kind: &str| format!("{path}{SEP}{name}{SEP}{kind}");
        self.run(&Spec {
            tier: Tier::T2,
            old_keys: &|o| vec![by_place(&o.cur.path, &o.cur.name, &o.cur.kind)],
            new_key: &|i| Some(by_place(&i.path, &i.name, i.kind.as_str())),
            accept: &|o, i| {
                o.cur.container == i.container || (same_sig(o, i) && same_body(min, o, i))
            },
            narrow: true,
        });

        let by_sig = |name: &str, kind: &str, sig: &str| format!("{name}{SEP}{kind}{SEP}{sig}");
        self.run(&Spec {
            tier: Tier::T3,
            old_keys: &|o| match &o.cur.sig_digest {
                Some(sig) if o.kind != EntityKind::Variable => {
                    vec![by_sig(&o.cur.name, &o.cur.kind, sig)]
                }
                _ => Vec::new(),
            },
            new_key: &|i| match &i.sig_digest {
                Some(sig) if !i.variable() => Some(by_sig(&i.name, i.kind.as_str(), sig)),
                _ => None,
            },
            accept: &|o, i| t3_accept(min, o, i),
            narrow: true,
        });

        self.run(&Spec {
            tier: Tier::T4,
            old_keys: &|o| match &o.cur.body_digest {
                Some(body) if o.kind != EntityKind::Variable && tokens(o.cur.body_tokens) => {
                    vec![body.clone()]
                }
                _ => Vec::new(),
            },
            new_key: &|i| match &i.body_digest {
                Some(body) if !i.variable() && tokens(i.body_tokens) => Some(body.clone()),
                _ => None,
            },
            accept: &|o, i| o.cur.path != i.path,
            narrow: false,
        });

        let by_body = |path: &str, container: &str, kind: &str, body: &str| {
            format!("{path}{SEP}{container}{SEP}{kind}{SEP}{body}")
        };
        self.run(&Spec {
            tier: Tier::T5,
            old_keys: &|o| match &o.cur.body_digest {
                Some(body) if tokens(o.cur.body_tokens) => {
                    vec![by_body(&o.cur.path, &o.cur.container, &o.cur.kind, body)]
                }
                _ => Vec::new(),
            },
            new_key: &|i| match &i.body_digest {
                Some(body) if tokens(i.body_tokens) => {
                    Some(by_body(&i.path, &i.container, i.kind.as_str(), body))
                }
                _ => None,
            },
            accept: &|o, i| o.cur.name != i.name,
            narrow: false,
        });
    }

    /// The tiers of the notes: T1, T1h, T2 for a note whose key changed with its path (a
    /// `loci.id` came or went), and M2 (the file moved and has the same hash).
    fn run_note_tiers(&mut self) {
        self.run(&Spec {
            tier: Tier::T1,
            old_keys: &|o| vec![o.cur.key.clone()],
            new_key: &|i| Some(i.key.clone()),
            accept: &|_, _| true,
            narrow: false,
        });
        self.run(&Spec {
            tier: Tier::T1H,
            old_keys: &|o| o.older_keys.clone(),
            new_key: &|i| Some(i.key.clone()),
            accept: &|_, _| true,
            narrow: false,
        });
        // A note with a `loci.id` has the same identity at another path. Another `loci.id` at the
        // same path is another note, so one side must have a path key.
        self.run(&Spec {
            tier: Tier::T2,
            old_keys: &|o| vec![o.cur.path.clone()],
            new_key: &|i| Some(i.path.clone()),
            accept: &|o, i| o.cur.key.starts_with("path:") || i.key.starts_with("path:"),
            narrow: false,
        });
        self.run(&Spec {
            tier: Tier::M2,
            old_keys: &|o| o.cur.body_digest.iter().cloned().collect(),
            new_key: &|i| i.body_digest.clone(),
            accept: &|o, i| o.cur.path != i.path,
            narrow: false,
        });
    }

    /// The tiers of the headings and blocks. The keys of the children hold the entity id of the
    /// note, so the notes must be bound first.
    fn run_child_tiers(&mut self) {
        let min = self.min_tokens;
        let tokens = move |t: Option<i64>| t.is_some_and(|t| t >= min);
        self.bind_heading_groups();
        self.run(&Spec {
            tier: Tier::T1,
            old_keys: &|o| vec![o.cur.key.clone()],
            new_key: &|i| Some(i.key.clone()),
            accept: &|_, _| true,
            narrow: false,
        });
        self.run(&Spec {
            tier: Tier::T1H,
            old_keys: &|o| o.older_keys.clone(),
            new_key: &|i| Some(i.key.clone()),
            accept: &|_, _| true,
            narrow: false,
        });
        // M3: the same block id, unique in the vault, in another note.
        self.run(&Spec {
            tier: Tier::M3,
            old_keys: &|o| match o.kind {
                EntityKind::Block => vec![o.cur.name.clone()],
                _ => Vec::new(),
            },
            new_key: &|i| (i.kind == EntityKind::Block).then(|| i.name.clone()),
            accept: &|o, i| o.parent_id.as_deref() != i.parent_key.as_deref(),
            narrow: false,
        });
        // M4: the same note and text, unique, under another parent.
        let by_text = |note: &str, text: &str| format!("{note}{SEP}{text}");
        self.run(&Spec {
            tier: Tier::M4,
            old_keys: &|o| match (o.kind, &o.parent_id) {
                (EntityKind::Heading, Some(note)) => vec![by_text(note, &o.cur.name)],
                _ => Vec::new(),
            },
            new_key: &|i| match (i.kind, &i.parent_key) {
                (EntityKind::Heading, Some(note)) => Some(by_text(note, &i.name)),
                _ => None,
            },
            accept: &|o, i| o.cur.container != i.container,
            narrow: false,
        });
        // M5: the same note and parent, the same section digest, unique, other text.
        let by_section =
            |note: &str, parent: &str, digest: &str| format!("{note}{SEP}{parent}{SEP}{digest}");
        self.run(&Spec {
            tier: Tier::M5,
            old_keys: &|o| match (o.kind, &o.parent_id, &o.cur.body_digest) {
                (EntityKind::Heading, Some(note), Some(digest)) if tokens(o.cur.body_tokens) => {
                    vec![by_section(note, &o.cur.container, digest)]
                }
                _ => Vec::new(),
            },
            new_key: &|i| match (i.kind, &i.parent_key, &i.body_digest) {
                (EntityKind::Heading, Some(note), Some(digest)) if tokens(i.body_tokens) => {
                    Some(by_section(note, &i.container, digest))
                }
                _ => None,
            },
            accept: &|o, i| o.cur.name != i.name,
            narrow: false,
        });
    }

    /// The duplicate group rule (design 7). A group is the headings with equal text under one
    /// parent. The rule applies to a group that has more than one member on either side.
    ///
    /// Bind the members by a section digest that is unique on both sides. When the size of the
    /// group is equal, bind the rest by ordinal. When the size changed, leave the rest: an old
    /// member that remains is `ambiguous`, and a new member that remains gets a new id. The tiers
    /// below skip every member of the group, so no other tier binds one by chance.
    fn bind_heading_groups(&mut self) {
        type Group = (String, String, String);
        let ordinal = |key: &str| {
            key.rsplit_once('#')
                .and_then(|(_, n)| n.parse::<u32>().ok())
                .unwrap_or(0)
        };
        let mut old_groups: BTreeMap<Group, Vec<usize>> = BTreeMap::new();
        for (o, old) in self.olds.iter().enumerate() {
            if old.kind != EntityKind::Heading || self.old_bound[o].is_some() {
                continue;
            }
            if let Some(note) = &old.parent_id {
                let group = (
                    note.clone(),
                    old.cur.container.clone(),
                    old.cur.name.clone(),
                );
                old_groups.entry(group).or_default().push(o);
            }
        }
        let mut new_groups: BTreeMap<Group, Vec<usize>> = BTreeMap::new();
        for (n, item) in self.items.iter().enumerate() {
            if item.kind != EntityKind::Heading || !item.is_child() || self.new_bound[n].is_some() {
                continue;
            }
            if let Some(note) = &item.parent_key {
                let group = (note.clone(), item.container.clone(), item.name.clone());
                new_groups.entry(group).or_default().push(n);
            }
        }
        for (group, mut olds) in old_groups {
            let Some(mut news) = new_groups.remove(&group) else {
                continue;
            };
            if olds.len() == 1 && news.len() == 1 {
                continue;
            }
            olds.sort_by_key(|&o| ordinal(&self.olds[o].cur.key));
            news.sort_by_key(|&n| ordinal(self.item(n).child_name.as_deref().unwrap_or("")));
            let same_size = olds.len() == news.len();
            let (all_olds, all_news) = (olds.clone(), news.clone());
            let mut pairs: Vec<(usize, usize)> = Vec::new();

            // A digest that exactly one old member and exactly one new member have.
            let mut by_digest: BTreeMap<&str, (Vec<usize>, Vec<usize>)> = BTreeMap::new();
            for &o in &olds {
                if let Some(d) = self.olds[o].cur.body_digest.as_deref() {
                    by_digest.entry(d).or_default().0.push(o);
                }
            }
            for &n in &news {
                if let Some(d) = self.item(n).body_digest.as_deref() {
                    by_digest.entry(d).or_default().1.push(n);
                }
            }
            for (_, (os, ns)) in by_digest {
                if let ([o], [n]) = (os.as_slice(), ns.as_slice()) {
                    if !self.vetoed(&self.olds[*o], self.item(*n)) {
                        pairs.push((*o, *n));
                    }
                }
            }
            olds.retain(|o| !pairs.iter().any(|p| p.0 == *o));
            news.retain(|n| !pairs.iter().any(|p| p.1 == *n));
            if same_size {
                // The rest bind by ordinal: the first with the first, and so on.
                for (&o, &n) in olds.iter().zip(&news) {
                    if !self.vetoed(&self.olds[o], self.item(n)) {
                        pairs.push((o, n));
                    }
                }
            }
            for &(o, n) in &pairs {
                let tier = if self.olds[o].cur.key == self.item(n).key {
                    Tier::T1
                } else {
                    Tier::M5
                };
                self.bind(o, n, tier);
            }
            // The members that remain have candidates but no bind.
            let left: BTreeSet<usize> = all_news
                .iter()
                .copied()
                .filter(|&n| self.new_bound[n].is_none())
                .collect();
            for &o in &all_olds {
                if self.old_bound[o].is_none() && !left.is_empty() {
                    self.hints[o] = Some((Tier::M5, left.clone()));
                }
                self.old_frozen[o] = true;
            }
            for &n in &all_news {
                self.new_frozen[n] = true;
            }
        }
    }

    /// Pair each vanished file with one new file (tier T2f). The map goes from the old path to
    /// the new path.
    fn pair_files(&self) -> HashMap<String, String> {
        // The paths that hold an unbound old entity, and the paths of every old entity.
        let mut gone: BTreeSet<&str> = BTreeSet::new();
        let mut known: HashSet<&str> = HashSet::new();
        for (o, old) in self.olds.iter().enumerate() {
            known.insert(old.cur.path.as_str());
            if !old.is_child()
                && self.old_bound[o].is_none()
                && !self.input.files.contains_key(&old.cur.path)
            {
                gone.insert(old.cur.path.as_str());
            }
        }
        if gone.is_empty() {
            return HashMap::new();
        }
        let old_files = self.input.old_files.as_ref();
        let fresh = |path: &str| {
            !known.contains(path) && old_files.is_none_or(|files| !files.contains_key(path))
        };
        // The new paths with a declaration that no old entity bound.
        let mut targets: BTreeSet<&str> = BTreeSet::new();
        for (n, item) in self.input.items.iter().enumerate() {
            if !item.is_child() && self.new_bound[n].is_none() && fresh(&item.path) {
                targets.insert(item.path.as_str());
            }
        }
        // A file without a declaration still has a hash. A module entity is a declaration, so
        // the targets above cover each file of a language that gives a module symbol. Add the
        // other fresh files, so that a hash can pair them.
        for path in self.input.files.keys() {
            if fresh(path) {
                targets.insert(path.as_str());
            }
        }
        let mut pairs: HashMap<String, String> = HashMap::new();

        // By the hash of the file.
        if let Some(old_files) = old_files {
            let mut old_by_hash: HashMap<&str, Vec<&str>> = HashMap::new();
            for path in &gone {
                if let Some(hash) = old_files.get(*path) {
                    old_by_hash.entry(hash.as_str()).or_default().push(path);
                }
            }
            let mut new_by_hash: HashMap<&str, Vec<&str>> = HashMap::new();
            for path in &targets {
                if let Some(Some(hash)) = self.input.files.get(*path) {
                    new_by_hash.entry(hash.as_str()).or_default().push(path);
                }
            }
            for (hash, from) in &old_by_hash {
                if let (&[from], Some(&[to])) =
                    (from.as_slice(), new_by_hash.get(hash).map(|v| v.as_slice()))
                {
                    pairs.insert(from.to_owned(), to.to_owned());
                }
            }
        }

        // By the probe: at least 2 entities of the old path bind uniquely to one new path by
        // T3 or T4, and none binds to another path.
        let paired_from: HashSet<&str> = pairs.keys().map(String::as_str).collect();
        let paired_to: HashSet<&str> = pairs.values().map(String::as_str).collect();
        let probe_to = |path: &str| targets.contains(path) && !paired_to.contains(path);
        let probe_from = |path: &str| gone.contains(path) && !paired_from.contains(path);
        let sig_key = |name: &str, kind: &str, sig: &str| format!("{name}{SEP}{kind}{SEP}{sig}");
        let min = self.min_tokens;
        let tokens = move |t: Option<i64>| t.is_some_and(|t| t >= min);
        let specs = [
            Spec {
                tier: Tier::T3,
                old_keys: &|o| match &o.cur.sig_digest {
                    Some(sig) if o.kind != EntityKind::Variable && probe_from(&o.cur.path) => {
                        vec![sig_key(&o.cur.name, &o.cur.kind, sig)]
                    }
                    _ => Vec::new(),
                },
                new_key: &|i| match &i.sig_digest {
                    Some(sig) if !i.variable() && probe_to(&i.path) => {
                        Some(sig_key(&i.name, i.kind.as_str(), sig))
                    }
                    _ => None,
                },
                accept: &|o, i| t3_accept(min, o, i),
                narrow: true,
            },
            Spec {
                tier: Tier::T4,
                old_keys: &|o| match &o.cur.body_digest {
                    Some(body)
                        if o.kind != EntityKind::Variable
                            && tokens(o.cur.body_tokens)
                            && probe_from(&o.cur.path) =>
                    {
                        vec![body.clone()]
                    }
                    _ => Vec::new(),
                },
                new_key: &|i| match &i.body_digest {
                    Some(body) if !i.variable() && tokens(i.body_tokens) && probe_to(&i.path) => {
                        Some(body.clone())
                    }
                    _ => None,
                },
                accept: &|o, i| o.cur.path != i.path,
                narrow: false,
            },
        ];
        // For each old path: the new path of each bound entity.
        let mut hits: BTreeMap<&str, BTreeMap<&str, HashSet<usize>>> = BTreeMap::new();
        for spec in &specs {
            let (found, _) = self.find(spec);
            for (o, n) in found {
                hits.entry(self.olds[o].cur.path.as_str())
                    .or_default()
                    .entry(self.item(n).path.as_str())
                    .or_default()
                    .insert(o);
            }
        }
        let mut by_target: HashMap<&str, Vec<&str>> = HashMap::new();
        let mut chosen: Vec<(&str, &str)> = Vec::new();
        for (from, to) in &hits {
            if to.len() == 1 {
                let (path, bound) = to.iter().next().expect("one target");
                if bound.len() >= 2 {
                    by_target.entry(path).or_default().push(from);
                    chosen.push((from, path));
                }
            }
        }
        for (from, to) in chosen {
            if by_target[to].len() == 1 {
                pairs.insert(from.to_owned(), to.to_owned());
            }
        }
        pairs
    }
}

// ---- Loading ----

fn parse_outcome(
    entity_id: String,
    outcome: String,
    tier: Option<String>,
    candidates: Option<String>,
    since_digest: String,
) -> Result<OutcomeRow, ScippyError> {
    let outcome = Outcome::parse(&outcome)
        .ok_or_else(|| ScippyError::Storage(format!("unknown outcome {outcome:?}")))?;
    let tier = match tier {
        Some(text) => Some(
            Tier::parse(&text)
                .ok_or_else(|| ScippyError::Storage(format!("unknown tier {text:?}")))?,
        ),
        None => None,
    };
    let candidates = match candidates {
        Some(text) => serde_json::from_str(&text)
            .map_err(|e| ScippyError::Storage(format!("bad candidates: {e}")))?,
        None => Vec::new(),
    };
    Ok(OutcomeRow {
        entity_id,
        outcome,
        tier,
        candidates,
        since_digest,
    })
}

fn load_olds(tx: &Transaction<'_>, family: Family) -> Result<Vec<Old>, ScippyError> {
    let family = family.as_str();
    let mut entities: Vec<(String, String, Option<String>)> = {
        let mut st = tx
            .prepare("SELECT id, kind, parent_id FROM entity WHERE family = ?1 ORDER BY id")
            .map_err(storage)?;
        let rows = st
            .query_map([family], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .map_err(storage)?;
        rows.collect::<Result<_, _>>().map_err(storage)?
    };
    let mut current: HashMap<String, Locator> = HashMap::new();
    let mut older: HashMap<String, Vec<String>> = HashMap::new();
    {
        let mut st = tx
            .prepare(
                "SELECT l.entity_id, l.seq, l.key, l.symbol, l.path, l.kind, l.name, l.container,
                        l.sig_digest, l.body_digest, l.body_tokens, l.input_digest
                 FROM locator l JOIN entity e ON e.id = l.entity_id
                 WHERE e.family = ?1 ORDER BY l.entity_id, l.seq",
            )
            .map_err(storage)?;
        let mut rows = st.query([family]).map_err(storage)?;
        while let Some(r) = rows.next().map_err(storage)? {
            let locator = Locator {
                entity_id: r.get(0).map_err(storage)?,
                seq: r.get(1).map_err(storage)?,
                key: r.get(2).map_err(storage)?,
                symbol: r.get(3).map_err(storage)?,
                path: r.get(4).map_err(storage)?,
                kind: r.get(5).map_err(storage)?,
                name: r.get(6).map_err(storage)?,
                container: r.get(7).map_err(storage)?,
                sig_digest: r.get(8).map_err(storage)?,
                body_digest: r.get(9).map_err(storage)?,
                body_tokens: r.get(10).map_err(storage)?,
                input_digest: r.get(11).map_err(storage)?,
            };
            if locator.seq == 0 {
                current.insert(locator.entity_id.clone(), locator);
            } else {
                older
                    .entry(locator.entity_id.clone())
                    .or_default()
                    .push(locator.key);
            }
        }
    }
    let mut outcomes: HashMap<String, OutcomeRow> = HashMap::new();
    {
        let mut st = tx
            .prepare(
                "SELECT o.entity_id, o.outcome, o.tier, o.candidates, o.since_digest
                 FROM outcome o JOIN entity e ON e.id = o.entity_id WHERE e.family = ?1",
            )
            .map_err(storage)?;
        let rows = st
            .query_map([family], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
            })
            .map_err(storage)?;
        for row in rows {
            let (id, outcome, tier, candidates, since): (String, String, _, _, String) =
                row.map_err(storage)?;
            outcomes.insert(
                id.clone(),
                parse_outcome(id, outcome, tier, candidates, since)?,
            );
        }
    }
    let mut olds = Vec::with_capacity(entities.len());
    for (id, kind, parent_id) in entities.drain(..) {
        let Some(cur) = current.remove(&id) else {
            continue;
        };
        let Some(kind) = EntityKind::parse(&kind) else {
            return Err(ScippyError::Storage(format!(
                "unknown entity kind {kind:?}"
            )));
        };
        olds.push(Old {
            older_keys: older.remove(&id).unwrap_or_default(),
            outcome: outcomes.remove(&id),
            id,
            kind,
            parent_id,
            cur,
        });
    }
    Ok(olds)
}

fn load_vetoes(tx: &Transaction<'_>) -> Result<HashMap<String, HashSet<String>>, ScippyError> {
    let mut st = tx
        .prepare("SELECT entity_id, key FROM veto")
        .map_err(storage)?;
    let rows = st
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
        .map_err(storage)?;
    let mut out: HashMap<String, HashSet<String>> = HashMap::new();
    for row in rows {
        let (id, key) = row.map_err(storage)?;
        out.entry(id).or_default().insert(key);
    }
    Ok(out)
}

// ---- Writing ----

fn locator_of(entity_id: &str, item: &Item, key: &str, digest: &str) -> Locator {
    Locator {
        entity_id: entity_id.into(),
        seq: 0,
        key: key.into(),
        symbol: item.symbol.clone(),
        path: item.path.clone(),
        kind: item.kind.as_str().into(),
        name: item.name.clone(),
        container: item.container.clone(),
        sig_digest: item.sig_digest.clone(),
        body_digest: item.body_digest.clone(),
        body_tokens: item.body_tokens,
        input_digest: digest.into(),
    }
}

fn insert_locator(tx: &Transaction<'_>, l: &Locator) -> Result<(), ScippyError> {
    tx.prepare_cached(
        "INSERT INTO locator (entity_id, seq, key, symbol, path, kind, name, container,
                              sig_digest, body_digest, body_tokens, input_digest)
         VALUES (?1, 0, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
    )
    .and_then(|mut st| {
        st.execute(params![
            l.entity_id,
            l.key,
            l.symbol,
            l.path,
            l.kind,
            l.name,
            l.container,
            l.sig_digest,
            l.body_digest,
            l.body_tokens,
            l.input_digest
        ])
    })
    .map_err(storage)?;
    Ok(())
}

/// Replace locator 0. The history stays.
fn update_locator(tx: &Transaction<'_>, l: &Locator) -> Result<(), ScippyError> {
    tx.prepare_cached(
        "UPDATE locator SET key = ?2, symbol = ?3, path = ?4, kind = ?5, name = ?6,
                container = ?7, sig_digest = ?8, body_digest = ?9, body_tokens = ?10,
                input_digest = ?11
         WHERE entity_id = ?1 AND seq = 0",
    )
    .and_then(|mut st| {
        st.execute(params![
            l.entity_id,
            l.key,
            l.symbol,
            l.path,
            l.kind,
            l.name,
            l.container,
            l.sig_digest,
            l.body_digest,
            l.body_tokens,
            l.input_digest
        ])
    })
    .map_err(storage)?;
    Ok(())
}

/// True when the two locators differ in a field that is not `input_digest`.
fn differs(a: &Locator, b: &Locator) -> bool {
    a.key != b.key
        || a.symbol != b.symbol
        || a.path != b.path
        || a.kind != b.kind
        || a.name != b.name
        || a.container != b.container
        || a.sig_digest != b.sig_digest
        || a.body_digest != b.body_digest
        || a.body_tokens != b.body_tokens
}

fn write_outcome(tx: &Transaction<'_>, row: &OutcomeRow) -> Result<(), ScippyError> {
    let candidates = if row.candidates.is_empty() {
        None
    } else {
        Some(
            serde_json::to_string(&row.candidates)
                .map_err(|e| ScippyError::Serialize(e.to_string()))?,
        )
    };
    tx.prepare_cached(
        "INSERT INTO outcome (entity_id, outcome, tier, candidates, since_digest)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(entity_id) DO UPDATE SET outcome = excluded.outcome,
           tier = excluded.tier, candidates = excluded.candidates,
           since_digest = excluded.since_digest",
    )
    .and_then(|mut st| {
        st.execute(params![
            row.entity_id,
            row.outcome.as_str(),
            row.tier.map(Tier::as_str),
            candidates,
            row.since_digest
        ])
    })
    .map_err(storage)?;
    Ok(())
}

fn clear_outcome(tx: &Transaction<'_>, entity_id: &str) -> Result<(), ScippyError> {
    tx.prepare_cached("DELETE FROM outcome WHERE entity_id = ?1")
        .and_then(|mut st| st.execute([entity_id]))
        .map_err(storage)?;
    Ok(())
}

fn mint(
    tx: &Transaction<'_>,
    ids: &dyn IdSource,
    family: Family,
    item: &Item,
    parent_id: Option<&str>,
    key: &str,
    digest: &str,
) -> Result<String, ScippyError> {
    let id = ids.new_id();
    tx.prepare_cached(
        "INSERT INTO entity (id, family, kind, parent_id, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
    )
    .and_then(|mut st| {
        st.execute(params![
            id,
            family.as_str(),
            item.kind.as_str(),
            parent_id,
            ids.now()
        ])
    })
    .map_err(storage)?;
    insert_locator(tx, &locator_of(&id, item, key, digest))?;
    Ok(id)
}

/// Apply one bind: update or push the locator, and set the outcome.
fn apply_bind(
    tx: &Transaction<'_>,
    old: &Old,
    item: &Item,
    key: &str,
    tier: Tier,
    digest: &str,
) -> Result<(), ScippyError> {
    let new = locator_of(&old.id, item, key, digest);
    let exact = matches!(tier, Tier::T1 | Tier::T1H | Tier::Param);
    let in_place = exact
        && old.cur.path == new.path
        && old.cur.container == new.container
        && old.cur.name == new.name;
    if in_place {
        if differs(&old.cur, &new) {
            update_locator(tx, &new)?;
        }
    } else {
        push_locator_in(tx, &new)?;
    }
    if exact {
        // A `moved` entity stays flagged until accept or reject. Any other state clears.
        if old
            .outcome
            .as_ref()
            .is_some_and(|o| o.outcome != Outcome::Moved)
        {
            clear_outcome(tx, &old.id)?;
        }
    } else {
        write_outcome(
            tx,
            &OutcomeRow {
                entity_id: old.id.clone(),
                outcome: Outcome::Moved,
                tier: Some(tier),
                candidates: Vec::new(),
                since_digest: digest.into(),
            },
        )?;
    }
    Ok(())
}

// ---- The entry point ----

/// Rebind the entities of one family to the items of a new run, in one transaction of the
/// identity store. The function sets `bound_input_digest` in the same transaction and commits.
///
/// An entity that no tier binds keeps its row, as `ambiguous` or `orphaned`. A declaration that
/// no entity claims gets a new id. The function is idempotent: the same input a second time
/// changes nothing but the recent digests.
pub fn rebind(
    identity: &mut Identity,
    input: &RebindInput,
    ids: &dyn IdSource,
) -> Result<RebindReport, ScippyError> {
    rebind_with(identity, input, ids, min_body_tokens())
}

/// [`rebind`] with a chosen minimum body size. The tests and the calibration use it.
pub fn rebind_with(
    identity: &mut Identity,
    input: &RebindInput,
    ids: &dyn IdSource,
    min_tokens: i64,
) -> Result<RebindReport, ScippyError> {
    rebind_commit(identity, input, ids, min_tokens, true)
}

/// [`rebind`] for a family that is not the last one of a publish. It leaves `bound_input_digest`
/// as it was. The last family of the publish sets the digest: see `publish`.
pub(crate) fn rebind_not_last(
    identity: &mut Identity,
    input: &RebindInput,
    ids: &dyn IdSource,
) -> Result<RebindReport, ScippyError> {
    rebind_commit(identity, input, ids, min_body_tokens(), false)
}

fn rebind_commit(
    identity: &mut Identity,
    input: &RebindInput,
    ids: &dyn IdSource,
    min_tokens: i64,
    set_bound: bool,
) -> Result<RebindReport, ScippyError> {
    let tx = identity
        .db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(storage)?;
    let report = rebind_in(&tx, input, ids, min_tokens, set_bound)?;
    tx.commit().map_err(storage)?;
    Ok(report)
}

fn rebind_in(
    tx: &Transaction<'_>,
    input: &RebindInput,
    ids: &dyn IdSource,
    min_tokens: i64,
    set_bound: bool,
) -> Result<RebindReport, ScippyError> {
    let digest = input.input_digest.as_str();
    let olds = load_olds(tx, input.family)?;
    let mut pool = Pool {
        input,
        items: input.items.clone(),
        children: false,
        old_frozen: vec![false; olds.len()],
        new_frozen: vec![false; input.items.len()],
        old_bound: vec![None; olds.len()],
        new_bound: vec![None; input.items.len()],
        hints: vec![None; olds.len()],
        olds,
        vetoes: load_vetoes(tx)?,
        min_tokens,
    };
    match input.family {
        Family::Code => pool.run_code_tiers(),
        Family::Note => pool.run_note_tiers(),
    }
    let mut report = RebindReport::default();

    // The entity id of each item. A bound item takes the id of its entity.
    let mut entity_of: Vec<Option<String>> = vec![None; input.items.len()];
    for (n, bound) in pool.new_bound.iter().enumerate() {
        if let Some(o) = bound {
            entity_of[n] = Some(pool.olds[*o].id.clone());
        }
    }
    // Write the binds of the parents.
    for (o, bound) in pool.old_bound.iter().enumerate() {
        let Some((n, tier)) = *bound else { continue };
        let item = pool.item(n);
        apply_bind(tx, &pool.olds[o], item, &item.key, tier, digest)?;
        *report.bound.entry(tier.as_str()).or_default() += 1;
    }
    // Mint an id for each unbound parent item.
    let mut by_key: HashMap<&str, usize> = HashMap::new();
    for (n, item) in input.items.iter().enumerate() {
        if item.is_child() {
            continue;
        }
        by_key.insert(item.key.as_str(), n);
        if entity_of[n].is_none() {
            entity_of[n] = Some(mint(tx, ids, input.family, item, None, &item.key, digest)?);
            report.minted += 1;
        }
    }

    if input.family == Family::Note {
        bind_note_children(
            tx,
            ids,
            &mut pool,
            &mut entity_of,
            &by_key,
            digest,
            &mut report,
        )?;
    }

    // The children bind when the parent has an entity and the name is equal.
    let mut child_old: HashMap<&str, usize> = HashMap::new();
    for (o, old) in pool.olds.iter().enumerate() {
        if old.is_child() {
            child_old.insert(old.cur.key.as_str(), o);
        }
    }
    let mut child_bound: HashSet<usize> = HashSet::new();
    for item in &input.items {
        if input.family != Family::Code {
            break;
        }
        let (Some(parent_key), Some(name)) = (&item.parent_key, &item.child_name) else {
            continue;
        };
        let Some(&parent) = by_key.get(parent_key.as_str()) else {
            continue;
        };
        let Some(parent_id) = entity_of[parent].as_deref() else {
            continue;
        };
        let key = child_key(parent_id, name);
        let found = child_old.get(key.as_str()).copied().filter(|&o| {
            !child_bound.contains(&o)
                && !pool
                    .vetoes
                    .get(&pool.olds[o].id)
                    .is_some_and(|keys| keys.contains(&key))
        });
        match found {
            Some(o) => {
                child_bound.insert(o);
                apply_bind(tx, &pool.olds[o], item, &key, Tier::Param, digest)?;
                *report.bound.entry(Tier::Param.as_str()).or_default() += 1;
            }
            None => {
                mint(tx, ids, input.family, item, Some(parent_id), &key, digest)?;
                report.minted += 1;
            }
        }
    }

    // The old entities that no tier bound.
    for (o, old) in pool.olds.iter().enumerate() {
        if pool.old_bound[o].is_some() || child_bound.contains(&o) {
            continue;
        }
        let candidates: Vec<String> = pool.hints[o]
            .as_ref()
            .filter(|_| input.family == Family::Note || !old.is_child())
            .map(|(_, found)| {
                found
                    .iter()
                    .filter(|&&n| pool.new_bound[n].is_none())
                    .map(|&n| pool.items[n].key.clone())
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect()
            })
            .unwrap_or_default();
        let (outcome, tier, candidates) = match (&pool.hints[o], candidates.is_empty()) {
            (Some((tier, _)), false) => (Outcome::Ambiguous, Some(*tier), candidates),
            _ => (Outcome::Orphaned, None, Vec::new()),
        };
        match outcome {
            Outcome::Ambiguous => report.ambiguous += 1,
            _ => report.orphaned += 1,
        }
        let same = old.outcome.as_ref().is_some_and(|row| {
            row.outcome == outcome && row.tier == tier && row.candidates == candidates
        });
        if !same {
            write_outcome(
                tx,
                &OutcomeRow {
                    entity_id: old.id.clone(),
                    outcome,
                    tier,
                    candidates,
                    since_digest: digest.into(),
                },
            )?;
        }
    }

    // The bookkeeping: the bound digest, the recent digests, and the orphan collection.
    let mut recent: Vec<String> = tx
        .query_row(
            "SELECT value FROM meta WHERE key = ?1",
            [META_RECENT_DIGESTS],
            |r| r.get::<_, String>(0),
        )
        .optional()
        .map_err(storage)?
        .map(|text| text.lines().map(str::to_owned).collect())
        .unwrap_or_default();
    if recent.last().map(String::as_str) != Some(digest) {
        recent.push(digest.to_owned());
    }
    if recent.len() > ORPHAN_GC_PUBLISHES {
        recent.drain(..recent.len() - ORPHAN_GC_PUBLISHES);
    }
    set_meta(tx, META_RECENT_DIGESTS, &recent.join("\n"))?;
    if set_bound {
        set_meta(tx, "bound_input_digest", digest)?;
    }
    set_meta(tx, items_meta_key(input.family), &input.items_digest())?;
    orphan_children(tx, input.family, digest)?;
    report.collected = collect_orphans(tx, input.family, &recent)?;
    Ok(report)
}

/// Bind the headings and blocks of the notes, and mint an id for each one that no entity claims.
/// The keys of these children hold the entity id of their note, so the notes are bound first.
fn bind_note_children(
    tx: &Transaction<'_>,
    ids: &dyn IdSource,
    pool: &mut Pool<'_>,
    entity_of: &mut [Option<String>],
    by_key: &HashMap<&str, usize>,
    digest: &str,
    report: &mut RebindReport,
) -> Result<(), ScippyError> {
    for n in 0..pool.items.len() {
        let item = &pool.items[n];
        let (Some(note_key), Some(suffix)) = (&item.parent_key, &item.child_name) else {
            continue;
        };
        let Some(note) = by_key
            .get(note_key.as_str())
            .and_then(|&p| entity_of[p].clone())
        else {
            continue;
        };
        let key = child_key(&note, suffix);
        let item = &mut pool.items[n];
        item.key = key;
        item.parent_key = Some(note);
    }
    pool.children = true;
    pool.run_child_tiers();
    for (o, bound) in pool.old_bound.clone().into_iter().enumerate() {
        let Some((n, tier)) = bound else { continue };
        let old = &pool.olds[o];
        if !old.is_child() {
            continue;
        }
        let item = &pool.items[n];
        apply_bind(tx, old, item, &item.key, tier, digest)?;
        // A block that moved to another note has a new parent.
        if old.parent_id != item.parent_key {
            tx.execute(
                "UPDATE entity SET parent_id = ?2 WHERE id = ?1",
                params![old.id, item.parent_key],
            )
            .map_err(storage)?;
        }
        entity_of[n] = Some(old.id.clone());
        *report.bound.entry(tier.as_str()).or_default() += 1;
    }
    for n in 0..pool.items.len() {
        let item = &pool.items[n];
        if !item.is_child() || item.key.is_empty() || pool.new_bound[n].is_some() {
            continue;
        }
        let parent = item.parent_key.clone();
        mint(
            tx,
            ids,
            Family::Note,
            item,
            parent.as_deref(),
            &item.key,
            digest,
        )?;
        report.minted += 1;
    }
    Ok(())
}

fn set_meta(tx: &Transaction<'_>, key: &str, value: &str) -> Result<(), ScippyError> {
    tx.execute(
        "INSERT INTO meta (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )
    .map_err(storage)?;
    Ok(())
}

/// Delete each unannotated orphan whose state is older than the recent digests.
fn collect_orphans(
    tx: &Transaction<'_>,
    family: Family,
    recent: &[String],
) -> Result<usize, ScippyError> {
    let recent: HashSet<&str> = recent.iter().map(String::as_str).collect();
    let dead: Vec<String> = {
        let mut st = tx
            .prepare(
                "SELECT o.entity_id, o.since_digest FROM outcome o
                 JOIN entity e ON e.id = o.entity_id
                 WHERE e.family = ?1 AND o.outcome = 'orphaned'
                   AND NOT EXISTS (SELECT 1 FROM annotation a WHERE a.entity_id = o.entity_id)",
            )
            .map_err(storage)?;
        let rows = st
            .query_map([family.as_str()], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })
            .map_err(storage)?;
        let mut dead = Vec::new();
        for row in rows {
            let (id, since) = row.map_err(storage)?;
            if !recent.contains(since.as_str()) {
                dead.push(id);
            }
        }
        dead
    };
    for id in &dead {
        for sql in [
            "DELETE FROM locator WHERE entity_id = ?1",
            "DELETE FROM outcome WHERE entity_id = ?1",
            "DELETE FROM veto WHERE entity_id = ?1",
            "DELETE FROM entity WHERE id = ?1",
        ] {
            tx.execute(sql, [id]).map_err(storage)?;
        }
    }
    Ok(dead.len())
}

/// Orphan each child whose parent is orphaned. A parent that `reject` orphaned leaves its
/// parameters `matched` in the store, and no rebinding runs when the input stays the same.
fn orphan_children(
    tx: &Transaction<'_>,
    family: Family,
    digest: &str,
) -> Result<usize, ScippyError> {
    tx.execute(
        "INSERT INTO outcome (entity_id, outcome, tier, candidates, since_digest)
         SELECT c.id, 'orphaned', NULL, NULL, ?1 FROM entity c
         JOIN outcome p ON p.entity_id = c.parent_id AND p.outcome = 'orphaned'
         WHERE c.family = ?2
           AND NOT EXISTS (SELECT 1 FROM outcome o WHERE o.entity_id = c.id AND o.outcome = 'orphaned')
         ON CONFLICT(entity_id) DO UPDATE SET outcome = 'orphaned', tier = NULL,
           candidates = NULL, since_digest = excluded.since_digest",
        params![digest, family.as_str()],
    )
    .map_err(storage)
}

/// Orphan the children of the orphaned entities of a family, without a rebinding. Publish calls
/// it when the store is already bound to the input, so a `reject` takes effect on parameters.
pub fn orphan_children_of_orphans(
    identity: &mut Identity,
    family: Family,
    input_digest: &str,
) -> Result<usize, ScippyError> {
    let tx = identity
        .db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(storage)?;
    let n = orphan_children(&tx, family, input_digest)?;
    tx.commit().map_err(storage)?;
    Ok(n)
}
