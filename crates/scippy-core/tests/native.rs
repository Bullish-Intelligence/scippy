//! The native engine seam, with a fake pack.
//!
//! The pack treats each line `ref <name>` of a `.txt` file as a reference to the document
//! `<name>.txt`, and each line `tag <name>` as a reference to a virtual symbol.
//! A line `ref <name>` that names a file that is not `.txt` makes a stub document.

use std::path::Path;

use protobuf::Message;
use scip::types::{Index, SymbolRole};
use scippy_core::{
    DeclKind, Descriptor, Engine, ExternalIdentity, Identity, IndexRequest, LanguagePack,
    NativeAnalysis, NativeDeclaration, NativeDocument, NativeInput, NativeOutput, NativeReference,
    NativeTarget, Package, ProjectModel, ProjectRelativePath, Reach, ReferenceRole, Scope,
    SymbolKind, VirtualSymbol, index_project,
};

struct Fake;

struct Model;

impl ProjectModel for Model {
    fn resolver_globals(&self, _: &ProjectRelativePath) -> Vec<(String, String)> {
        Vec::new()
    }
    fn module_identity(&self, path: &ProjectRelativePath) -> Option<Identity> {
        Some(Identity {
            reach: Reach::Global,
            descriptors: vec![Descriptor::Namespace(path.as_str().to_string())],
            kind: SymbolKind::File,
            display_name: path.as_str().to_string(),
        })
    }
    fn identity(
        &self,
        path: &ProjectRelativePath,
        _: &[Scope],
        d: &Scope,
        _: &[Scope],
    ) -> Identity {
        Identity {
            reach: Reach::Global,
            descriptors: vec![
                Descriptor::Namespace(path.as_str().to_string()),
                Descriptor::Term(d.name.clone()),
            ],
            kind: SymbolKind::Key,
            display_name: d.name.clone(),
        }
    }
}

impl LanguagePack for Fake {
    fn id(&self) -> &'static str {
        "fake"
    }
    fn scip_language(&self) -> &'static str {
        "Text"
    }
    fn accepts(&self, path: &Path) -> bool {
        path.extension().is_some_and(|e| e == "txt")
    }
    fn engine(&self) -> Option<Engine<'_>> {
        Some(Engine::Native(self))
    }
}

impl NativeAnalysis for Fake {
    fn default_package(&self) -> Package {
        Package {
            manager: "fake".into(),
            name: "p".into(),
            version: String::new(),
        }
    }
    fn project_model(&self, _: &Path) -> Box<dyn ProjectModel> {
        Box::new(Model)
    }
    fn analyze(&self, input: &NativeInput) -> NativeOutput {
        let mut out = NativeOutput::default();
        let mut stubs = Vec::new();
        for source in input.sources {
            let mut references = Vec::new();
            let mut declarations = Vec::new();
            let mut at = 0u32;
            for line in source.text.lines() {
                let len = line.len() as u32;
                let span = (at, at + len);
                at += len + 1;
                let mut parts = line.split(' ');
                match (parts.next(), parts.next()) {
                    (Some("decl"), Some(name)) => declarations.push(NativeDeclaration {
                        span,
                        name: name.to_string(),
                        kind: DeclKind::Section,
                        of_module: false,
                    }),
                    (Some("ref"), Some(name)) => {
                        let file = input
                            .files
                            .iter()
                            .find(|f| f.path.as_str().starts_with(name));
                        let targets = file
                            .map(|f| {
                                if !f.path.as_str().ends_with(".txt") {
                                    stubs.push(f.path.clone());
                                }
                                vec![NativeTarget::Declaration(f.path.clone(), (0, 0))]
                            })
                            .unwrap_or_default();
                        references.push(NativeReference {
                            span,
                            role: if name.starts_with("read") {
                                ReferenceRole::Read
                            } else {
                                ReferenceRole::Reference
                            },
                            targets,
                        });
                    }
                    (Some("tag"), Some(name)) => {
                        out.virtual_symbols.push(VirtualSymbol {
                            key: name.to_string(),
                            identity: ExternalIdentity {
                                package: Package {
                                    manager: "fake".into(),
                                    name: "tags".into(),
                                    version: String::new(),
                                },
                                descriptors: vec![Descriptor::Namespace(name.to_string())],
                                kind: SymbolKind::Key,
                                display_name: name.to_string(),
                            },
                        });
                        references.push(NativeReference {
                            span,
                            role: ReferenceRole::Reference,
                            targets: vec![NativeTarget::Virtual(name.to_string())],
                        });
                    }
                    _ => {}
                }
            }
            out.documents.push(NativeDocument {
                path: source.path.clone(),
                stub: false,
                module: true,
                declarations,
                references,
            });
        }
        for path in stubs {
            out.documents.push(NativeDocument {
                path,
                stub: true,
                module: true,
                declarations: Vec::new(),
                references: Vec::new(),
            });
        }
        out
    }
}

fn index(dir: &Path) -> Index {
    let out = dir.join("out.scip");
    let report = index_project(&IndexRequest::new(dir, &out), &[&Fake]).unwrap();
    assert_eq!(report.references_unresolved, 1);
    Index::parse_from_bytes(&std::fs::read(&out).unwrap()).unwrap()
}

#[test]
fn a_native_pack_declares_resolves_and_gets_external_and_stub_symbols() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("a.txt"),
        "decl head\nref b\ntag urgent\nref missing\nref readme.png\n",
    )
    .unwrap();
    std::fs::write(dir.path().join("b.txt"), "decl other\n").unwrap();
    std::fs::write(dir.path().join("readme.png"), "").unwrap();
    let index = index(dir.path());
    let doc = |p: &str| {
        index
            .documents
            .iter()
            .find(|d| d.relative_path == p)
            .unwrap()
    };
    let a = doc("a.txt");
    // The definitions: the module and one heading. Then three references that resolve.
    let defs: Vec<_> = a
        .occurrences
        .iter()
        .filter(|o| o.symbol_roles & SymbolRole::Definition as i32 != 0)
        .collect();
    assert_eq!(defs.len(), 2);
    let refs: Vec<_> = a
        .occurrences
        .iter()
        .filter(|o| o.symbol_roles & SymbolRole::Definition as i32 == 0)
        .collect();
    assert_eq!(refs.len(), 3);
    assert!(refs.iter().any(|o| o.symbol.ends_with("`b.txt`/")));
    assert!(refs.iter().any(|o| o.symbol.ends_with("urgent/")));
    assert!(
        refs.iter()
            .any(|o| o.symbol_roles & SymbolRole::ReadAccess as i32 != 0)
    );
    // The tag is an external symbol, and the stub defines the file.
    assert_eq!(index.external_symbols.len(), 1);
    let stub = doc("readme.png");
    assert_eq!(stub.language, "");
    assert_eq!(stub.occurrences.len(), 1);
}
