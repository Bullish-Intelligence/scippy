//! The public API of the identity store: resolve a locator, and add, list, remove, review, accept,
//! and reject annotations. The design is in `METALAYER_IDENTITY.md`, sections 6 and 8.
//!
//! Every method reads the identity store only. None reads the facts store.

use std::collections::HashMap;
use std::fmt;

use super::BindOutcome;
use super::*;

/// One entity that a locator matches. [`IdentityError::AmbiguousLocator`] lists them.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct LocatorMatch {
    pub id: String,
    pub kind: EntityKind,
    /// The locator text of the entity: `path::Outer.Inner`, `note.md#H1/H2`, and so on.
    pub display: String,
}

/// A failure of an annotation operation.
#[derive(Debug)]
#[non_exhaustive]
pub enum IdentityError {
    /// No entity matches the locator.
    NoEntity(String),
    /// Several entities match the locator.
    AmbiguousLocator {
        locator: String,
        matches: Vec<LocatorMatch>,
    },
    /// The declaration that a candidate names has an annotation, so `accept` cannot delete its id.
    CandidateAnnotated {
        /// The ambiguous entity.
        entity_id: String,
        /// The key of the candidate.
        candidate: String,
        /// The entity that holds the annotation.
        annotated_id: String,
    },
    /// No annotation has this id.
    NoAnnotation(String),
    /// The call does not fit the state of the entity, or an argument is wrong.
    BadArgument(String),
    /// The store failed.
    Store(ScippyError),
}

impl IdentityError {
    /// The stable error kind of the CLI contract.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::NoEntity(_) => "no-entity",
            Self::AmbiguousLocator { .. } => "ambiguous-locator",
            Self::CandidateAnnotated { .. } => "candidate-annotated",
            Self::NoAnnotation(_) => "not-found",
            Self::BadArgument(_) => "bad-argument",
            Self::Store(ScippyError::Io { .. }) => "io",
            Self::Store(_) => "storage",
        }
    }
}

impl fmt::Display for IdentityError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoEntity(locator) => write!(f, "no entity matches {locator:?}"),
            Self::AmbiguousLocator { locator, matches } => {
                write!(f, "{locator:?} matches {} entities:", matches.len())?;
                for m in matches {
                    write!(f, " {} {} {};", m.id, m.kind.as_str(), m.display)?;
                }
                write!(f, " use an entity id")
            }
            Self::CandidateAnnotated {
                entity_id,
                candidate,
                annotated_id,
            } => write!(
                f,
                "cannot bind {entity_id} to {candidate:?}: the entity {annotated_id} of that declaration has an annotation"
            ),
            Self::NoAnnotation(id) => write!(f, "no annotation has the id {id:?}"),
            Self::BadArgument(text) => f.write_str(text),
            Self::Store(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for IdentityError {}

impl From<ScippyError> for IdentityError {
    fn from(error: ScippyError) -> Self {
        Self::Store(error)
    }
}

/// The result of [`IdentityStore::probe`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum IdentityProbe {
    /// No file exists at the path.
    Missing,
    /// The file is not an identity store.
    NotIdentity,
    /// An identity store with this format version.
    Version(i32),
}

/// An entity with everything the store knows about it.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct EntityDetail {
    pub entity: Entity,
    /// The locator history, current first.
    pub locators: Vec<Locator>,
    /// `None` means `matched`.
    pub outcome: Option<OutcomeRow>,
    /// The keys that the entity must not bind to.
    pub vetoes: Vec<String>,
    /// Oldest first.
    pub annotations: Vec<Annotation>,
}

impl EntityDetail {
    /// The binding state of the entity.
    pub fn state(&self) -> BindOutcome {
        self.outcome
            .as_ref()
            .map_or(BindOutcome::Matched, |o| o.outcome)
    }

    /// The current locator.
    pub fn locator(&self) -> Option<&Locator> {
        self.locators.first()
    }
}

/// The filter of [`IdentityStore::list`]. Each set field narrows the result.
#[derive(Debug, Default, Clone)]
#[non_exhaustive]
pub struct AnnotationFilter {
    /// A locator of one entity. An entity id is the usual form.
    pub entity: Option<String>,
    /// The path of the current locator.
    pub path: Option<String>,
    /// The binding state of the entity.
    pub status: Option<BindOutcome>,
}

/// One annotation with its entity.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct AnnotationEntry {
    pub annotation: Annotation,
    pub entity: Entity,
    /// The current locator. An orphaned entity shows its last locator.
    pub locator: Option<Locator>,
    pub state: BindOutcome,
    pub tier: Option<Tier>,
}

/// One entity that needs review: it is `moved`, `ambiguous`, or `orphaned`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct StatusEntry {
    pub entity: Entity,
    pub locator: Option<Locator>,
    pub outcome: OutcomeRow,
    /// The number of annotations on the entity.
    pub annotations: usize,
}

/// What [`IdentityStore::accept`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum AcceptAction {
    /// A `moved` entity: the flag is gone.
    Cleared,
    /// An `ambiguous` entity took the locator of a candidate.
    Bound {
        /// The key of the candidate.
        candidate: String,
        /// The id minted for the declaration. The store deleted it.
        removed_entity: String,
    },
}

/// The result of [`IdentityStore::accept`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct AcceptResult {
    pub entity_id: String,
    pub action: AcceptAction,
}

impl Locator {
    /// The locator text of an entity of `kind` at this locator, in the form that
    /// [`IdentityStore::resolve`] accepts: `path::Outer.Inner`, `note.md#H1/H2`, `note.md^id`.
    /// A module shows as `path::`.
    pub fn display(&self, kind: EntityKind) -> String {
        let mut names = self.containers();
        match kind {
            EntityKind::Note => self.path.clone(),
            EntityKind::Module => format!("{}::", self.path),
            EntityKind::Heading => {
                names.push(&self.name);
                format!("{}#{}", self.path, names.join("/"))
            }
            EntityKind::Block => format!("{}^{}", self.path, block_id(&self.name)),
            _ => {
                names.push(&self.name);
                format!("{}::{}", self.path, names.join("."))
            }
        }
    }

    /// The fields as JSON, without the entity id and without `seq`. The container is a list of
    /// display names. The snapshot of an annotation uses this form.
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "key": self.key,
            "symbol": self.symbol,
            "path": self.path,
            "kind": self.kind,
            "name": self.name,
            "container": self.containers(),
            "sig_digest": self.sig_digest,
            "body_digest": self.body_digest,
            "body_tokens": self.body_tokens,
            "input_digest": self.input_digest,
        })
    }
}

/// The text of a block id without the `^`.
fn block_id(name: &str) -> &str {
    name.strip_prefix('^').unwrap_or(name)
}

fn bad(text: impl Into<String>) -> IdentityError {
    IdentityError::BadArgument(text.into())
}

fn ids_of(
    db: &Connection,
    sql: &str,
    params: impl rusqlite::Params,
) -> Result<Vec<String>, ScippyError> {
    let mut statement = db.prepare(sql).map_err(storage)?;
    let rows = statement.query_map(params, |r| r.get(0)).map_err(storage)?;
    rows.collect::<Result<_, _>>().map_err(storage)
}

fn annotation_count(db: &Connection, entity_id: &str) -> Result<i64, ScippyError> {
    db.query_row(
        "SELECT COUNT(*) FROM annotation WHERE entity_id = ?1",
        [entity_id],
        |r| r.get(0),
    )
    .map_err(storage)
}

impl IdentityStore {
    /// Look at a path without opening it for use. It never writes, creates, or migrates.
    pub fn probe(path: impl AsRef<Path>) -> Result<IdentityProbe, ScippyError> {
        let path = path.as_ref();
        if !path.exists() {
            return Ok(IdentityProbe::Missing);
        }
        let Ok(db) = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY) else {
            return Ok(IdentityProbe::NotIdentity);
        };
        // A file that SQLite cannot read fails here, so it is not a store of ours.
        let Ok(found) = header(&db) else {
            return Ok(IdentityProbe::NotIdentity);
        };
        if found.application_id != IDENTITY_APPLICATION_ID {
            return Ok(IdentityProbe::NotIdentity);
        }
        Ok(IdentityProbe::Version(found.version))
    }

    // ---- Resolve ----

    /// Find the one entity that a locator names.
    ///
    /// A locator is one of these forms, tried in this order:
    /// an entity id; a symbol string (the symbol or the key of a current locator);
    /// `path::Outer.Inner` (`path::` names the module of the file); `note.md` (a note, or the
    /// module of a code file); `note.md#H1/H2` (a heading, with its ancestors); `note.md^id`
    /// (a block). The first form that matches decides.
    ///
    /// The match reads the current locator of each entity. An orphaned entity matches with its
    /// last locator, but only when no other entity matches. Several matches give
    /// [`IdentityError::AmbiguousLocator`]. No match gives [`IdentityError::NoEntity`].
    pub fn resolve(&self, locator: &str) -> Result<Entity, IdentityError> {
        let mut found = self.find(locator)?;
        if found.len() > 1 {
            let mut live = Vec::new();
            for entity in &found {
                if self.get_outcome(&entity.id)?.map(|o| o.outcome) != Some(BindOutcome::Orphaned) {
                    live.push(entity.clone());
                }
            }
            if !live.is_empty() {
                found = live;
            }
        }
        match found.len() {
            0 => Err(IdentityError::NoEntity(locator.into())),
            1 => Ok(found.remove(0)),
            _ => {
                let mut matches = Vec::new();
                for entity in found {
                    let display = self
                        .current_locator(&entity.id)?
                        .map_or_else(String::new, |l| l.display(entity.kind));
                    matches.push(LocatorMatch {
                        id: entity.id,
                        kind: entity.kind,
                        display,
                    });
                }
                Err(IdentityError::AmbiguousLocator {
                    locator: locator.into(),
                    matches,
                })
            }
        }
    }

    fn entities(&self, ids: Vec<String>) -> Result<Vec<Entity>, ScippyError> {
        let mut out = Vec::new();
        for id in ids {
            if let Some(entity) = self.get_entity(&id)? {
                out.push(entity);
            }
        }
        Ok(out)
    }

    /// The entities whose current locator has this path, with the locator.
    fn at_path(&self, path: &str) -> Result<Vec<(Entity, Locator)>, ScippyError> {
        let ids = ids_of(
            &self.db,
            "SELECT entity_id FROM locator WHERE seq = 0 AND path = ?1 ORDER BY entity_id",
            [path],
        )?;
        let mut out = Vec::new();
        for entity in self.entities(ids)? {
            if let Some(locator) = self.current_locator(&entity.id)? {
                out.push((entity, locator));
            }
        }
        Ok(out)
    }

    fn find(&self, text: &str) -> Result<Vec<Entity>, ScippyError> {
        if let Some(entity) = self.get_entity(text)? {
            return Ok(vec![entity]);
        }
        let by_symbol = ids_of(
            &self.db,
            "SELECT entity_id FROM locator WHERE seq = 0 AND (symbol = ?1 OR key = ?1)
             ORDER BY entity_id",
            [text],
        )?;
        if !by_symbol.is_empty() {
            return self.entities(by_symbol);
        }
        if let Some((path, rest)) = text.split_once("::") {
            let mut out = Vec::new();
            for (entity, locator) in self.at_path(path)? {
                let hit = if rest.is_empty() {
                    entity.kind == EntityKind::Module
                } else {
                    let mut names = locator.containers();
                    names.push(&locator.name);
                    !matches!(
                        entity.kind,
                        EntityKind::Module
                            | EntityKind::Note
                            | EntityKind::Heading
                            | EntityKind::Block
                    ) && names.join(".") == rest
                };
                if hit {
                    out.push(entity);
                }
            }
            if !out.is_empty() {
                return Ok(out);
            }
        }
        let (path, tail) = match text.find(['#', '^']) {
            Some(at) => (&text[..at], Some((&text[at..=at], &text[at + 1..]))),
            None => (text, None),
        };
        let mut out = Vec::new();
        for (entity, locator) in self.at_path(path)? {
            let hit = match (tail, entity.kind) {
                (None, EntityKind::Note | EntityKind::Module) => true,
                (Some(("#", rest)), EntityKind::Heading) => {
                    let mut names = locator.containers();
                    names.push(&locator.name);
                    names.join("/") == rest
                }
                (Some(("^", rest)), EntityKind::Block) => block_id(&locator.name) == rest,
                _ => false,
            };
            if hit {
                out.push(entity);
            }
        }
        Ok(out)
    }

    // ---- Read ----

    fn load_detail(&self, entity: Entity) -> Result<EntityDetail, ScippyError> {
        Ok(EntityDetail {
            locators: self.locators(&entity.id)?,
            outcome: self.get_outcome(&entity.id)?,
            vetoes: self.vetoes(&entity.id)?,
            annotations: self.list_annotations(&entity.id)?,
            entity,
        })
    }

    /// Resolve a locator and read the entity with its locators, outcome, vetoes, and annotations.
    pub fn detail(&self, locator: &str) -> Result<EntityDetail, IdentityError> {
        let entity = self.resolve(locator)?;
        Ok(self.load_detail(entity)?)
    }

    /// List annotations. The order is the path of the current locator, the name, `created_at`,
    /// and the annotation id.
    pub fn list(&self, filter: &AnnotationFilter) -> Result<Vec<AnnotationEntry>, IdentityError> {
        let annotations = match &filter.entity {
            Some(locator) => {
                let entity = self.resolve(locator)?;
                self.list_annotations(&entity.id)?
            }
            None => {
                let mut statement = self
                    .db
                    .prepare(
                        "SELECT id, entity_id, created_at, author, tag, body, snapshot
                         FROM annotation",
                    )
                    .map_err(storage)?;
                let rows = statement.query_map([], annotation_row).map_err(storage)?;
                rows.collect::<Result<Vec<_>, _>>().map_err(storage)?
            }
        };
        let mut seen: HashMap<String, (Entity, Option<Locator>, Option<OutcomeRow>)> =
            HashMap::new();
        let mut out = Vec::new();
        for annotation in annotations {
            if !seen.contains_key(&annotation.entity_id) {
                let Some(entity) = self.get_entity(&annotation.entity_id)? else {
                    continue;
                };
                let locator = self.current_locator(&entity.id)?;
                let outcome = self.get_outcome(&entity.id)?;
                seen.insert(entity.id.clone(), (entity, locator, outcome));
            }
            let (entity, locator, outcome) = &seen[&annotation.entity_id];
            let state = outcome.as_ref().map_or(BindOutcome::Matched, |o| o.outcome);
            if filter.status.is_some_and(|wanted| wanted != state) {
                continue;
            }
            if let Some(path) = &filter.path
                && locator.as_ref().is_none_or(|l| &l.path != path)
            {
                continue;
            }
            out.push(AnnotationEntry {
                annotation,
                entity: entity.clone(),
                locator: locator.clone(),
                state,
                tier: outcome.as_ref().and_then(|o| o.tier),
            });
        }
        out.sort_by(|a, b| {
            let key = |e: &AnnotationEntry| {
                (
                    e.locator.as_ref().map(|l| (l.path.clone(), l.name.clone())),
                    e.annotation.created_at.clone(),
                    e.annotation.id.clone(),
                )
            };
            key(a).cmp(&key(b))
        });
        Ok(out)
    }

    /// The entities that need review: each one with an outcome row. The order is the path of
    /// the current locator, the name, and the entity id.
    pub fn status(&self) -> Result<Vec<StatusEntry>, IdentityError> {
        let ids = ids_of(
            &self.db,
            "SELECT entity_id FROM outcome ORDER BY entity_id",
            [],
        )?;
        let mut out = Vec::new();
        for entity in self.entities(ids)? {
            let Some(outcome) = self.get_outcome(&entity.id)? else {
                continue;
            };
            out.push(StatusEntry {
                locator: self.current_locator(&entity.id)?,
                annotations: usize::try_from(annotation_count(&self.db, &entity.id)?)
                    .unwrap_or_default(),
                outcome,
                entity,
            });
        }
        out.sort_by(|a, b| {
            let key = |e: &StatusEntry| {
                (
                    e.locator.as_ref().map(|l| (l.path.clone(), l.name.clone())),
                    e.entity.id.clone(),
                )
            };
            key(a).cmp(&key(b))
        });
        Ok(out)
    }

    // ---- Write ----

    /// Add an annotation to the entity that `locator` names. The snapshot holds the current
    /// locator. The body must not be empty.
    pub fn add(
        &mut self,
        ids: &dyn IdSource,
        locator: &str,
        body: &str,
        tag: Option<&str>,
        author: Option<&str>,
    ) -> Result<Annotation, IdentityError> {
        if body.trim().is_empty() {
            return Err(bad("the annotation body is empty"));
        }
        let entity = self.resolve(locator)?;
        let snapshot = self
            .current_locator(&entity.id)?
            .map_or(serde_json::Value::Null, |l| l.to_json());
        let annotation = Annotation {
            id: ids.new_id(),
            entity_id: entity.id,
            created_at: ids.now(),
            author: author.map(Into::into),
            tag: tag.map(Into::into),
            body: body.into(),
            snapshot: snapshot.to_string(),
        };
        self.insert_annotation(&annotation)?;
        Ok(annotation)
    }

    /// Delete one annotation by id. The result is the deleted annotation.
    pub fn remove(&mut self, annotation_id: &str) -> Result<Annotation, IdentityError> {
        let annotation = self
            .get_annotation(annotation_id)?
            .ok_or_else(|| IdentityError::NoAnnotation(annotation_id.into()))?;
        self.delete_annotation(annotation_id)?;
        Ok(annotation)
    }

    /// Accept the state of one entity.
    ///
    /// A `moved` entity loses its flag. An `ambiguous` entity needs `candidate`, one of its
    /// candidate keys: it takes the locator of the declaration with that key. The id minted for
    /// that declaration is deleted, with the ids of its children. If one of them has an
    /// annotation, the call fails with [`IdentityError::CandidateAnnotated`] and changes nothing.
    /// A `matched` or `orphaned` entity has nothing to accept.
    pub fn accept(
        &mut self,
        locator: &str,
        candidate: Option<&str>,
    ) -> Result<AcceptResult, IdentityError> {
        let entity = self.resolve(locator)?;
        let Some(row) = self.get_outcome(&entity.id)? else {
            return Err(bad(format!("{} is matched: nothing to accept", entity.id)));
        };
        let action = match row.outcome {
            BindOutcome::Moved => {
                if candidate.is_some() {
                    return Err(bad("--candidate applies to an ambiguous entity only"));
                }
                self.clear_outcome(&entity.id)?;
                AcceptAction::Cleared
            }
            BindOutcome::Ambiguous => {
                let Some(candidate) = candidate else {
                    return Err(bad(format!(
                        "{} is ambiguous: pass --candidate with one of: {}",
                        entity.id,
                        row.candidates.join(", ")
                    )));
                };
                if !row.candidates.iter().any(|c| c == candidate) {
                    return Err(bad(format!(
                        "{candidate:?} is not a candidate of {}; the candidates are: {}",
                        entity.id,
                        row.candidates.join(", ")
                    )));
                }
                self.bind_candidate(&entity.id, candidate)?
            }
            _ => {
                return Err(bad(format!(
                    "{} is {}: nothing to accept",
                    entity.id,
                    row.outcome.as_str()
                )));
            }
        };
        Ok(AcceptResult {
            entity_id: entity.id,
            action,
        })
    }

    fn bind_candidate(
        &mut self,
        entity_id: &str,
        candidate: &str,
    ) -> Result<AcceptAction, IdentityError> {
        let tx = self.db.transaction().map_err(storage)?;
        let holders = ids_of(
            &tx,
            "SELECT entity_id FROM locator WHERE seq = 0 AND key = ?1 AND entity_id != ?2
             ORDER BY entity_id",
            params![candidate, entity_id],
        )?;
        let removed = match holders.as_slice() {
            [one] => one.clone(),
            [] => {
                return Err(bad(format!(
                    "no declaration has the key {candidate:?}; run the publish again"
                )));
            }
            _ => {
                return Err(bad(format!(
                    "{} declarations have the key {candidate:?}: {}",
                    holders.len(),
                    holders.join(", ")
                )));
            }
        };
        let mut doomed = ids_of(
            &tx,
            "SELECT id FROM entity WHERE parent_id = ?1 ORDER BY id",
            [&removed],
        )?;
        doomed.insert(0, removed.clone());
        for id in &doomed {
            if annotation_count(&tx, id)? > 0 {
                return Err(IdentityError::CandidateAnnotated {
                    entity_id: entity_id.into(),
                    candidate: candidate.into(),
                    annotated_id: id.clone(),
                });
            }
        }
        let mut locator = tx
            .query_row(
                &format!("SELECT {LOCATOR_COLUMNS} FROM locator WHERE entity_id = ?1 AND seq = 0"),
                [&removed],
                locator_row,
            )
            .map_err(storage)?;
        locator.entity_id = entity_id.into();
        push_locator_in(&tx, &locator)?;
        for id in &doomed {
            for table in ["locator", "outcome", "veto"] {
                tx.execute(&format!("DELETE FROM {table} WHERE entity_id = ?1"), [id])
                    .map_err(storage)?;
            }
            tx.execute("DELETE FROM entity WHERE id = ?1", [id])
                .map_err(storage)?;
        }
        tx.execute("DELETE FROM outcome WHERE entity_id = ?1", [entity_id])
            .map_err(storage)?;
        tx.commit().map_err(storage)?;
        Ok(AcceptAction::Bound {
            candidate: candidate.into(),
            removed_entity: removed,
        })
    }

    /// Accept every `moved` entity that a tier bound. The result is the ids, sorted.
    pub fn accept_all(&mut self, tier: Tier) -> Result<Vec<String>, IdentityError> {
        let tx = self.db.transaction().map_err(storage)?;
        let ids = ids_of(
            &tx,
            "SELECT entity_id FROM outcome WHERE outcome = 'moved' AND tier = ?1
             ORDER BY entity_id",
            [tier.as_str()],
        )?;
        tx.execute(
            "DELETE FROM outcome WHERE outcome = 'moved' AND tier = ?1",
            [tier.as_str()],
        )
        .map_err(storage)?;
        tx.commit().map_err(storage)?;
        Ok(ids)
    }

    /// Reject the state of one entity. The result is the keys that the entity must not bind to.
    ///
    /// A `moved` entity gets a veto on its new key, goes back to its old locator, and becomes
    /// `orphaned`. An `ambiguous` entity gets a veto on each candidate and becomes `orphaned`.
    /// The entity keeps its annotations. A `matched` or `orphaned` entity has nothing to reject.
    pub fn reject(&mut self, locator: &str) -> Result<Vec<String>, IdentityError> {
        let entity = self.resolve(locator)?;
        let Some(row) = self.get_outcome(&entity.id)? else {
            return Err(bad(format!("{} is matched: nothing to reject", entity.id)));
        };
        let tx = self.db.transaction().map_err(storage)?;
        let vetoed = match row.outcome {
            BindOutcome::Moved => {
                let locators = {
                    let mut statement = tx
                        .prepare(&format!(
                            "SELECT {LOCATOR_COLUMNS} FROM locator WHERE entity_id = ?1
                             ORDER BY seq"
                        ))
                        .map_err(storage)?;
                    let rows = statement
                        .query_map([&entity.id], locator_row)
                        .map_err(storage)?;
                    rows.collect::<Result<Vec<_>, _>>().map_err(storage)?
                };
                let Some(current) = locators.first() else {
                    return Err(bad(format!("{} has no locator", entity.id)));
                };
                if locators.len() > 1 {
                    // Drop locator 0. The negative step keeps the primary key free of clashes.
                    tx.execute(
                        "DELETE FROM locator WHERE entity_id = ?1 AND seq = 0",
                        [&entity.id],
                    )
                    .map_err(storage)?;
                    tx.execute(
                        "UPDATE locator SET seq = -seq WHERE entity_id = ?1",
                        [&entity.id],
                    )
                    .map_err(storage)?;
                    tx.execute(
                        "UPDATE locator SET seq = -seq - 1 WHERE entity_id = ?1",
                        [&entity.id],
                    )
                    .map_err(storage)?;
                }
                vec![current.key.clone()]
            }
            BindOutcome::Ambiguous => row.candidates.clone(),
            _ => {
                return Err(bad(format!(
                    "{} is {}: nothing to reject",
                    entity.id,
                    row.outcome.as_str()
                )));
            }
        };
        for key in &vetoed {
            tx.execute(
                "INSERT OR IGNORE INTO veto (entity_id, key) VALUES (?1, ?2)",
                params![entity.id, key],
            )
            .map_err(storage)?;
        }
        tx.execute(
            "UPDATE outcome SET outcome = 'orphaned', tier = NULL, candidates = NULL
             WHERE entity_id = ?1",
            [&entity.id],
        )
        .map_err(storage)?;
        tx.commit().map_err(storage)?;
        Ok(vetoed)
    }
}
