//! A file that changes while the run reads it becomes a failed file with no facts.

use std::path::Path;

use scippy_core::{
    DeclKind, Descriptor, Engine, Identity, IndexRequest, LanguagePack, NativeAnalysis,
    NativeDeclaration, NativeDocument, NativeInput, NativeOutput, Package, ProjectModel,
    ProjectRelativePath, Reach, Scope, SymbolKind, file_status, index_project_with_snapshot,
};

/// A native pack that declares one heading for each file. While it analyzes, it rewrites the
/// file `moving.txt`, as an editor does.
struct Moving;

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
            out.documents.push(NativeDocument {
                path: source.path.clone(),
                stub: false,
                module: true,
                declarations: vec![NativeDeclaration {
                    span: (0, 4),
                    name: "head".into(),
                    kind: DeclKind::Section,
                    of_module: false,
                }],
                references: Vec::new(),
            });
        }
        out
    }
}

#[test]
fn a_file_that_changes_during_the_run_is_failed_and_loses_its_facts() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("project");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("moving.txt"), "head one\n").unwrap();
    std::fs::write(root.join("steady.txt"), "head two\n").unwrap();
    let request = IndexRequest::new(&root, dir.path().join("index.scip"));
    let (report, snapshot) = index_project_with_snapshot(&request, &[&Moving]).unwrap();

    assert_eq!(report.files_indexed, 1);
    assert_eq!(report.files_skipped, 1);
    let paths: Vec<_> = snapshot.documents.iter().map(|d| d.path.as_str()).collect();
    assert_eq!(paths, ["steady.txt"]);
    assert_eq!(snapshot.lost_files.len(), 1);
    let lost = &snapshot.lost_files[0];
    assert_eq!(lost.path, "moving.txt");
    assert_eq!(lost.reason, "changed-during-run");
    assert_eq!(lost.status, "failed");
    assert_eq!(lost.status, file_status("changed-during-run"));
    assert_eq!(lost.pack.as_deref(), Some("moving"));
    // The hash is the hash of the bytes that the run read: "head one\n".
    assert_eq!(lost.sha256.as_deref().map(str::len), Some(64));
    assert!(!snapshot.input_complete);
    let diagnostic = snapshot
        .diagnostics
        .iter()
        .find(|d| d.code == "changed-during-run")
        .expect("a diagnostic");
    assert_eq!(diagnostic.file.as_deref(), Some("moving.txt"));
    // The SCIP index holds no document for the file either.
    assert!(!snapshot.scip.windows(10).any(|w| w == b"moving.txt"));
}
