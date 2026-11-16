//! A file that changes while the run works keeps its facts from the first read. The run reads
//! each file once. At the end it reports the file as stale.

use std::path::Path;

use scippy_core::{
    DeclKind, Descriptor, Engine, Identity, IndexRequest, LanguagePack, NativeAnalysis,
    NativeDeclaration, NativeDocument, NativeInput, NativeOutput, Package, ProjectModel,
    ProjectRelativePath, Reach, Scope, SymbolKind, index_project_with_snapshot,
};

fn sha256_hex(text: &str) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(text.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// A native pack that declares one heading for each file. While it analyzes, it rewrites the
/// file `moving.txt`, as an editor does. It records the text that it got for each file.
struct Moving {
    seen: std::sync::Mutex<Vec<(String, String)>>,
}

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

impl LanguagePack for Moving {
    fn id(&self) -> &'static str {
        "moving"
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

impl NativeAnalysis for Moving {
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
        std::fs::write(input.root.join("moving.txt"), "changed after the read\n").unwrap();
        let mut out = NativeOutput::default();
        for source in input.sources {
            self.seen
                .lock()
                .unwrap()
                .push((source.path.as_str().to_owned(), source.text.to_string()));
            out.documents.push(NativeDocument {
                path: source.path.clone(),
                stub: false,
                module: true,
                outline: None,
                declarations: vec![NativeDeclaration {
                    span: (0, 4),
                    name: "head".into(),
                    kind: DeclKind::Section,
                    of_module: false,
                    outline: None,
                }],
                references: Vec::new(),
            });
        }
        out
    }
}

#[test]
fn a_file_that_changes_during_the_run_keeps_its_first_content_and_is_stale() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("project");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("moving.txt"), "head one\n").unwrap();
    std::fs::write(root.join("steady.txt"), "head two\n").unwrap();
    let request = IndexRequest::new(&root, dir.path().join("index.scip"));
    let pack = Moving {
        seen: Default::default(),
    };
    let (report, snapshot) = index_project_with_snapshot(&request, &[&pack]).unwrap();

    // The pack got the first content, and the run kept both files.
    let seen = pack.seen.lock().unwrap().clone();
    assert!(seen.contains(&("moving.txt".to_owned(), "head one\n".to_owned())));
    assert_eq!(report.files_indexed, 2);
    assert_eq!(report.files_skipped, 0);
    assert!(snapshot.lost_files.is_empty());
    let paths: Vec<_> = snapshot.documents.iter().map(|d| d.path.as_str()).collect();
    assert_eq!(paths, ["moving.txt", "steady.txt"]);
    let moving = &snapshot.documents[0];
    assert_eq!(moving.source, "head one\n");
    assert_eq!(
        moving.sha256.as_deref(),
        Some(sha256_hex("head one\n").as_str())
    );
    assert!(moving.stale);
    assert!(!snapshot.documents[1].stale);
    assert_eq!(report.stale_files, ["moving.txt"]);
    // The file is not lost: the input stays complete.
    assert!(snapshot.input_complete, "{:?}", snapshot.diagnostics);
    let diagnostic = snapshot
        .diagnostics
        .iter()
        .find(|d| d.code == "stale-file")
        .expect("a diagnostic");
    assert_eq!(diagnostic.file.as_deref(), Some("moving.txt"));
    // The SCIP index holds the document.
    assert!(snapshot.scip.windows(10).any(|w| w == b"moving.txt"));
}
