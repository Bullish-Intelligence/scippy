//! The emitter: turn documents, occurrences, and symbol facts into SCIP bytes.
//!
//! The model below is the input of the emitter. A later binding stage fills it.
//! The emitter checks the model, sorts it, and writes it in a fixed order.
//! The same facts in any order give the same bytes.

use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use protobuf::Message;
use scip::types::{
    Document as ScipDocument, Index, Metadata, MultiLineRange, Occurrence as ScipOccurrence,
    PositionEncoding, ProtocolVersion, SingleLineRange, SymbolInformation, SymbolRole,
    TextEncoding, ToolInfo, occurrence::Typed_range, symbol_information::Kind,
};

use crate::error::ScippyError;
use crate::identity::SymbolId;
use crate::positions::{LineIndex, Range};
use crate::project::ProjectRelativePath;

/// Why an occurrence exists.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Role {
    Reference,
    Definition,
    Import,
}

/// A symbol occurrence. The span is in bytes of the document source.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Occurrence {
    pub start_byte: u32,
    pub end_byte: u32,
    pub symbol: SymbolId,
    pub role: Role,
}

/// The kind of a symbol. Use `Unspecified` when the language pack does not know.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SymbolKind {
    Unspecified,
    Module,
    Class,
    Function,
    Method,
    Variable,
    Constant,
    Field,
}

/// Metadata of a symbol that the document defines.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SymbolInfo {
    pub symbol: SymbolId,
    pub kind: SymbolKind,
    pub display_name: String,
}

/// One source file.
#[derive(Clone, Debug)]
pub struct Document {
    pub path: ProjectRelativePath,
    /// The SCIP language name, for example `Python`.
    pub language: String,
    /// The text that the occurrence spans refer to. It is not written to the index.
    /// It may stay empty when the document has no occurrences.
    pub source: String,
    pub occurrences: Vec<Occurrence>,
    pub symbols: Vec<SymbolInfo>,
}

/// Everything the emitter writes.
#[derive(Clone, Debug)]
pub struct IndexData {
    /// The absolute project root. The emitter writes it as a `file://` URI.
    pub project_root: PathBuf,
    pub documents: Vec<Document>,
}

/// Check, sort, and serialize the index.
pub fn encode(index: IndexData) -> Result<Vec<u8>, ScippyError> {
    build(index)?
        .write_to_bytes()
        .map_err(|e| ScippyError::Serialize(e.to_string()))
}

/// Serialize the index and write it to `output` through a temporary file and a rename.
/// A failure leaves no truncated file at `output`.
pub fn write_index(index: IndexData, output: &Path) -> Result<(), ScippyError> {
    let bytes = encode(index)?;
    write_atomic(output, &bytes)
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), ScippyError> {
    let io = |source| ScippyError::Io {
        path: path.to_path_buf(),
        source,
    };
    let name = path
        .file_name()
        .ok_or_else(|| io(std::io::Error::other("output path has no file name")))?;
    let temp = path.with_file_name(format!(
        ".{}.scippy-tmp-{}",
        name.to_string_lossy(),
        std::process::id()
    ));
    let result = (|| {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::rename(&temp, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result.map_err(io)
}

fn build(mut index: IndexData) -> Result<Index, ScippyError> {
    index.documents.sort_by(|a, b| a.path.cmp(&b.path));
    if let Some(pair) = index.documents.windows(2).find(|w| w[0].path == w[1].path) {
        return Err(ScippyError::DuplicateDocument {
            path: pair[0].path.as_str().to_string(),
        });
    }

    let mut check = Collisions::default();
    let documents = index
        .documents
        .into_iter()
        .map(|document| build_document(document, &mut check))
        .collect::<Result<Vec<_>, _>>()?;

    Ok(Index {
        metadata: Some(Metadata {
            version: ProtocolVersion::UnspecifiedProtocolVersion.into(),
            tool_info: Some(ToolInfo {
                name: "scippy".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
                ..Default::default()
            })
            .into(),
            project_root: root_uri(&index.project_root)?,
            text_document_encoding: TextEncoding::UTF8.into(),
            ..Default::default()
        })
        .into(),
        documents,
        ..Default::default()
    })
}

/// Facts that must hold across documents.
#[derive(Default)]
struct Collisions {
    /// Where each global symbol has its definition.
    definitions: BTreeMap<SymbolId, (String, Range)>,
    /// Which document holds the symbol information of each global symbol.
    infos: BTreeMap<SymbolId, String>,
    /// Which document uses each local symbol.
    locals: BTreeMap<SymbolId, String>,
}

fn collision(symbol: &SymbolId, detail: String) -> ScippyError {
    ScippyError::SymbolCollision {
        symbol: symbol.as_str().to_string(),
        detail,
    }
}

fn build_document(document: Document, check: &mut Collisions) -> Result<ScipDocument, ScippyError> {
    let path = document.path.as_str();
    let lines = LineIndex::new(&document.source);
    if document.source.len() > i32::MAX as usize {
        return Err(ScippyError::InvalidRange {
            path: path.to_string(),
            start_byte: 0,
            end_byte: 0,
            reason: "source text is larger than 2 GiB",
        });
    }

    let mut occurrences = Vec::with_capacity(document.occurrences.len());
    for o in document.occurrences {
        let range =
            lines
                .range(o.start_byte, o.end_byte)
                .map_err(|reason| ScippyError::InvalidRange {
                    path: path.to_string(),
                    start_byte: o.start_byte,
                    end_byte: o.end_byte,
                    reason,
                })?;
        occurrences.push((range, o.symbol, o.role));
    }
    occurrences.sort();
    occurrences.dedup();

    for (range, symbol, role) in &occurrences {
        if symbol.is_local() {
            let first = check
                .locals
                .entry(symbol.clone())
                .or_insert_with(|| path.to_string());
            if first != path {
                return Err(collision(
                    symbol,
                    format!("local symbol used in {first} and in {path}"),
                ));
            }
        } else if *role == Role::Definition {
            let here = (path.to_string(), *range);
            let first = check
                .definitions
                .entry(symbol.clone())
                .or_insert_with(|| here.clone());
            if *first != here {
                let detail = format!(
                    "defined at {}:{}:{} and at {}:{}:{}",
                    first.0,
                    first.1.start_line,
                    first.1.start_character,
                    here.0,
                    here.1.start_line,
                    here.1.start_character
                );
                return Err(collision(symbol, detail));
            }
        }
    }

    let mut symbols: BTreeMap<SymbolId, SymbolInfo> = BTreeMap::new();
    for info in document.symbols {
        match symbols.get(&info.symbol) {
            Some(first) if *first != info => {
                return Err(collision(
                    &info.symbol,
                    format!("two different symbol records in {path}"),
                ));
            }
            Some(_) => {}
            None => {
                if !info.symbol.is_local() {
                    let first = check
                        .infos
                        .entry(info.symbol.clone())
                        .or_insert_with(|| path.to_string());
                    if first != path {
                        return Err(collision(
                            &info.symbol,
                            format!("symbol record in {first} and in {path}"),
                        ));
                    }
                }
                symbols.insert(info.symbol.clone(), info);
            }
        }
    }

    Ok(ScipDocument {
        language: document.language,
        relative_path: path.to_string(),
        occurrences: occurrences
            .into_iter()
            .map(|(range, symbol, role)| scip_occurrence(range, symbol, role))
            .collect(),
        symbols: symbols.into_values().map(scip_symbol).collect(),
        position_encoding: PositionEncoding::UTF8CodeUnitOffsetFromLineStart.into(),
        ..Default::default()
    })
}

fn scip_occurrence(range: Range, symbol: SymbolId, role: Role) -> ScipOccurrence {
    let typed = if range.start_line == range.end_line {
        Typed_range::SingleLineRange(SingleLineRange {
            line: range.start_line as i32,
            start_character: range.start_character as i32,
            end_character: range.end_character as i32,
            ..Default::default()
        })
    } else {
        Typed_range::MultiLineRange(MultiLineRange {
            start_line: range.start_line as i32,
            start_character: range.start_character as i32,
            end_line: range.end_line as i32,
            end_character: range.end_character as i32,
            ..Default::default()
        })
    };
    let roles = match role {
        Role::Reference => 0,
        Role::Definition => SymbolRole::Definition as i32,
        Role::Import => SymbolRole::Import as i32,
    };
    ScipOccurrence {
        symbol: symbol.as_str().to_string(),
        symbol_roles: roles,
        typed_range: Some(typed),
        ..Default::default()
    }
}

fn scip_symbol(info: SymbolInfo) -> SymbolInformation {
    let kind = match info.kind {
        SymbolKind::Unspecified => Kind::UnspecifiedKind,
        SymbolKind::Module => Kind::Module,
        SymbolKind::Class => Kind::Class,
        SymbolKind::Function => Kind::Function,
        SymbolKind::Method => Kind::Method,
        SymbolKind::Variable => Kind::Variable,
        SymbolKind::Constant => Kind::Constant,
        SymbolKind::Field => Kind::Field,
    };
    SymbolInformation {
        symbol: info.symbol.as_str().to_string(),
        kind: kind.into(),
        display_name: info.display_name,
        ..Default::default()
    }
}

/// Write an absolute path as a `file://` URI. Percent-encode every byte that is not unreserved.
fn root_uri(root: &Path) -> Result<String, ScippyError> {
    let text = match root.to_str() {
        Some(text) if root.is_absolute() => text,
        _ => {
            return Err(ScippyError::InvalidRoot {
                path: root.to_path_buf(),
                reason: "the project root must be an absolute UTF-8 path".to_string(),
            });
        }
    };
    let mut uri = String::from("file://");
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' => {
                uri.push(byte as char)
            }
            _ => uri.push_str(&format!("%{byte:02X}")),
        }
    }
    Ok(uri)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::{Descriptor, IdentityBuilder, Package};

    fn sym(name: &str) -> SymbolId {
        IdentityBuilder::new(
            "scippy",
            Package {
                manager: "pip".into(),
                name: "proj".into(),
                version: "0.1".into(),
            },
        )
        .symbol(&[
            Descriptor::Namespace("m".into()),
            Descriptor::Method {
                name: name.into(),
                disambiguator: String::new(),
            },
        ])
        .unwrap()
    }

    fn occ(source: &str, needle: &str, nth: usize, symbol: SymbolId, role: Role) -> Occurrence {
        let start = source.match_indices(needle).nth(nth).unwrap().0 as u32;
        Occurrence {
            start_byte: start,
            end_byte: start + needle.len() as u32,
            symbol,
            role,
        }
    }

    fn info(symbol: SymbolId) -> SymbolInfo {
        SymbolInfo {
            display_name: "x".into(),
            symbol,
            kind: SymbolKind::Function,
        }
    }

    fn doc(
        path: &str,
        source: &str,
        occurrences: Vec<Occurrence>,
        symbols: Vec<SymbolInfo>,
    ) -> Document {
        Document {
            path: ProjectRelativePath::new(path).unwrap(),
            language: "Python".into(),
            source: source.into(),
            occurrences,
            symbols,
        }
    }

    fn data(documents: Vec<Document>) -> IndexData {
        IndexData {
            project_root: "/proj/root".into(),
            documents,
        }
    }

    fn parse(bytes: &[u8]) -> Index {
        Index::parse_from_bytes(bytes).unwrap()
    }

    #[test]
    fn writes_metadata_first_with_explicit_encodings() {
        let bytes = encode(data(vec![doc("a.py", "", vec![], vec![])])).unwrap();
        assert_eq!(bytes[0], 0x0a, "field 1 (metadata) comes first");
        let index = parse(&bytes);
        let meta = index.metadata.as_ref().unwrap();
        assert_eq!(meta.project_root, "file:///proj/root");
        assert_eq!(meta.tool_info.name, "scippy");
        assert_eq!(meta.tool_info.version, env!("CARGO_PKG_VERSION"));
        assert_eq!(
            meta.text_document_encoding.enum_value(),
            Ok(TextEncoding::UTF8)
        );
        let document = &index.documents[0];
        assert_eq!(
            document.position_encoding.enum_value(),
            Ok(PositionEncoding::UTF8CodeUnitOffsetFromLineStart)
        );
        assert_eq!(document.language, "Python");
        assert_eq!(document.relative_path, "a.py");
    }

    #[test]
    fn uses_typed_ranges_and_never_the_packed_range() {
        let source = "def f(x):\n    return (x +\n            x)\n";
        let f = sym("f");
        let multiline = Occurrence {
            start_byte: source.find("x +").unwrap() as u32,
            end_byte: source.rfind("x)").unwrap() as u32 + 1,
            symbol: f.clone(),
            role: Role::Reference,
        };
        let bytes = encode(data(vec![doc(
            "a.py",
            source,
            vec![occ(source, "f", 1, f.clone(), Role::Definition), multiline],
            vec![],
        )]))
        .unwrap();
        let occurrences = &parse(&bytes).documents[0].occurrences;
        assert!(
            occurrences
                .iter()
                .all(|o| o.range.is_empty() && o.enclosing_range.is_empty())
        );
        assert!(matches!(
            occurrences[0].typed_range,
            Some(Typed_range::SingleLineRange(SingleLineRange {
                line: 0,
                start_character: 4,
                end_character: 5,
                ..
            }))
        ));
        assert!(matches!(
            occurrences[1].typed_range,
            Some(Typed_range::MultiLineRange(MultiLineRange {
                start_line: 1,
                start_character: 12,
                end_line: 2,
                end_character: 13,
                ..
            }))
        ));
        assert_eq!(occurrences[0].symbol_roles, SymbolRole::Definition as i32);
        assert_eq!(occurrences[1].symbol_roles, 0);
    }

    #[test]
    fn ranges_use_utf8_byte_columns_on_non_ascii_lines() {
        // Each of "é", "日" and "🙂" is longer than one byte.
        let source = "é = '日🙂'; name = 1\nname = é\n";
        let n = sym("name");
        let bytes = encode(data(vec![doc(
            "a.py",
            source,
            vec![
                occ(source, "name", 0, n.clone(), Role::Definition),
                occ(source, "name", 1, n, Role::Reference),
            ],
            vec![],
        )]))
        .unwrap();
        let columns: Vec<_> = parse(&bytes).documents[0]
            .occurrences
            .iter()
            .map(|o| match &o.typed_range {
                Some(Typed_range::SingleLineRange(r)) => {
                    (r.line, r.start_character, r.end_character)
                }
                other => panic!("{other:?}"),
            })
            .collect();
        // Bytes before the first `name`: é(2) ' = '(3) quote(1) 日(3) 🙂(4) quote(1) ';'(1) ' '(1) = 16.
        assert_eq!(columns, [(0, 16, 20), (1, 0, 4)]);
    }

    #[test]
    fn sorts_documents_occurrences_and_symbols() {
        let source = "a b c\n";
        let (a, b, c) = (sym("a"), sym("b"), sym("c"));
        let mut scrambled = vec![
            occ(source, "c", 0, c.clone(), Role::Reference),
            occ(source, "a", 0, a.clone(), Role::Definition),
            occ(source, "b", 0, b.clone(), Role::Reference),
        ];
        // The same span with two symbols sorts by symbol.
        scrambled.push(occ(source, "c", 0, a.clone(), Role::Reference));
        let documents = vec![
            doc("z.py", "", vec![], vec![]),
            doc(
                "m/b.py",
                source,
                scrambled,
                vec![info(c), info(a.clone()), info(b)],
            ),
            doc("a.py", "", vec![], vec![]),
        ];
        let index = parse(&encode(data(documents)).unwrap());
        let paths: Vec<_> = index
            .documents
            .iter()
            .map(|d| d.relative_path.as_str())
            .collect();
        assert_eq!(paths, ["a.py", "m/b.py", "z.py"]);
        let document = &index.documents[1];
        let columns: Vec<_> = document
            .occurrences
            .iter()
            .map(|o| match &o.typed_range {
                Some(Typed_range::SingleLineRange(r)) => {
                    (r.start_character, o.symbol.rsplit('/').next().unwrap())
                }
                _ => unreachable!(),
            })
            .collect();
        assert_eq!(
            columns,
            [(0, "a()."), (2, "b()."), (4, "a()."), (4, "c().")]
        );
        let symbols: Vec<_> = document.symbols.iter().map(|s| s.symbol.clone()).collect();
        let mut sorted = symbols.clone();
        sorted.sort();
        assert_eq!(symbols, sorted);
    }

    #[test]
    fn the_same_facts_in_any_order_give_the_same_bytes() {
        let source = "a b\n";
        let (a, b) = (sym("a"), sym("b"));
        let forward = data(vec![
            doc(
                "x.py",
                source,
                vec![
                    occ(source, "a", 0, a.clone(), Role::Definition),
                    occ(source, "b", 0, b.clone(), Role::Reference),
                ],
                vec![info(a.clone())],
            ),
            doc(
                "y.py",
                source,
                vec![occ(source, "a", 0, a.clone(), Role::Reference)],
                vec![],
            ),
        ]);
        let mut backward = forward.clone();
        backward.documents.reverse();
        for d in &mut backward.documents {
            d.occurrences.reverse();
        }
        assert_eq!(encode(forward).unwrap(), encode(backward).unwrap());
    }

    #[test]
    fn removes_duplicate_occurrences() {
        let source = "a\n";
        let a = sym("a");
        let one = occ(source, "a", 0, a, Role::Reference);
        let bytes = encode(data(vec![doc(
            "a.py",
            source,
            vec![one.clone(), one],
            vec![],
        )]))
        .unwrap();
        assert_eq!(parse(&bytes).documents[0].occurrences.len(), 1);
    }

    #[test]
    fn rejects_two_definitions_of_one_symbol() {
        let source = "a a\n";
        let a = sym("a");
        let same_doc = doc(
            "a.py",
            source,
            vec![
                occ(source, "a", 0, a.clone(), Role::Definition),
                occ(source, "a", 1, a.clone(), Role::Definition),
            ],
            vec![],
        );
        assert!(matches!(
            encode(data(vec![same_doc])),
            Err(ScippyError::SymbolCollision { .. })
        ));

        let one = doc(
            "a.py",
            source,
            vec![occ(source, "a", 0, a.clone(), Role::Definition)],
            vec![],
        );
        let two = doc(
            "b.py",
            source,
            vec![occ(source, "a", 0, a, Role::Definition)],
            vec![],
        );
        assert!(matches!(
            encode(data(vec![one, two])),
            Err(ScippyError::SymbolCollision { .. })
        ));
    }

    #[test]
    fn rejects_conflicting_symbol_records() {
        let a = sym("a");
        let other = SymbolInfo {
            kind: SymbolKind::Class,
            ..info(a.clone())
        };
        let conflict = doc("a.py", "", vec![], vec![info(a.clone()), other]);
        assert!(matches!(
            encode(data(vec![conflict])),
            Err(ScippyError::SymbolCollision { .. })
        ));

        // An identical record twice in one document is merged.
        let twice = doc("a.py", "", vec![], vec![info(a.clone()), info(a.clone())]);
        assert_eq!(
            parse(&encode(data(vec![twice])).unwrap()).documents[0]
                .symbols
                .len(),
            1
        );

        // A record in two documents is a collision.
        let (one, two) = (
            doc("a.py", "", vec![], vec![info(a.clone())]),
            doc("b.py", "", vec![], vec![info(a)]),
        );
        assert!(matches!(
            encode(data(vec![one, two])),
            Err(ScippyError::SymbolCollision { .. })
        ));
    }

    #[test]
    fn a_local_symbol_stays_in_one_document() {
        let source = "x\n";
        let local = |path| {
            doc(
                path,
                source,
                vec![occ(source, "x", 0, SymbolId::local(1), Role::Definition)],
                vec![],
            )
        };
        // The same local id in two documents is two symbols, but one document may not share it.
        assert!(matches!(
            encode(data(vec![local("a.py"), local("b.py")])),
            Err(ScippyError::SymbolCollision { .. })
        ));
        assert!(encode(data(vec![local("a.py")])).is_ok());
    }

    #[test]
    fn rejects_duplicate_documents_and_bad_ranges() {
        let dup = data(vec![
            doc("a.py", "", vec![], vec![]),
            doc("a.py", "", vec![], vec![]),
        ]);
        assert!(matches!(
            encode(dup),
            Err(ScippyError::DuplicateDocument { .. })
        ));

        let a = sym("a");
        let bad = |start, end| {
            let o = Occurrence {
                start_byte: start,
                end_byte: end,
                symbol: a.clone(),
                role: Role::Reference,
            };
            encode(data(vec![doc("a.py", "é\n", vec![o], vec![])]))
        };
        assert!(
            matches!(bad(0, 9), Err(ScippyError::InvalidRange { .. })),
            "past the end"
        );
        assert!(
            matches!(bad(1, 2), Err(ScippyError::InvalidRange { .. })),
            "inside é"
        );
        assert!(
            matches!(bad(2, 2), Err(ScippyError::InvalidRange { .. })),
            "empty"
        );
        assert!(bad(0, 2).is_ok());
    }

    #[test]
    fn root_uri_is_absolute_and_percent_encoded() {
        assert_eq!(
            root_uri(Path::new("/tmp/a b/ü")).unwrap(),
            "file:///tmp/a%20b/%C3%BC"
        );
        assert!(root_uri(Path::new("relative/dir")).is_err());
    }

    #[test]
    fn atomic_write_replaces_the_file_and_leaves_no_temporary_file() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("index.scip");
        fs::write(&out, b"old").unwrap();
        write_index(data(vec![doc("a.py", "", vec![], vec![])]), &out).unwrap();
        assert_eq!(parse(&fs::read(&out).unwrap()).documents.len(), 1);
        let names: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, ["index.scip"]);
    }

    #[test]
    fn a_failed_write_leaves_the_old_output_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("index.scip");
        fs::write(&out, b"old").unwrap();
        // A collision stops the run before any file operation.
        let source = "a a\n";
        let a = sym("a");
        let bad = doc(
            "a.py",
            source,
            vec![
                occ(source, "a", 0, a.clone(), Role::Definition),
                occ(source, "a", 1, a, Role::Definition),
            ],
            vec![],
        );
        assert!(write_index(data(vec![bad]), &out).is_err());
        assert_eq!(fs::read(&out).unwrap(), b"old");
        // A missing directory fails without a temporary file.
        let missing = dir.path().join("none").join("index.scip");
        assert!(matches!(
            write_index(data(vec![]), &missing),
            Err(ScippyError::Io { .. })
        ));
    }
}
