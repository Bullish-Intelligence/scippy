//! Gate 6 conformance checks of the core.
//!
//! The core is generic: it names no language in its control flow (concept, section 11,
//! condition 11). Invalid binding rules fail the run with a message and write no file
//! (condition 12).

use std::fs;
use std::path::{Path, PathBuf};

use scippy_core::{
    Analysis, BindingRules, Descriptor, ExtractionSpec, Identity, IndexRequest, LanguagePack,
    Manifest, Package, ProjectModel, ProjectRelativePath, Reach, Scope, SymbolKind, index_project,
};

/// Words that name a language of a pack. The core must not hold them outside test code.
const LANGUAGE_WORDS: [&str; 5] = ["python", "typescript", "tsx", "pyproject", "tsconfig"];

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// The lines of a source file without comments and without `#[cfg(test)]` items.
fn production_lines(text: &str) -> Vec<(usize, String)> {
    let mut kept = Vec::new();
    let mut depth: Option<i64> = None;
    let mut armed = false;
    for (number, line) in text.lines().enumerate() {
        let code = line.split("//").next().unwrap_or("");
        if let Some(open) = depth.as_mut() {
            *open += code.matches('{').count() as i64 - code.matches('}').count() as i64;
            if *open <= 0 {
                depth = None;
            }
            continue;
        }
        if armed {
            // The item that follows `#[cfg(test)]`. It ends at its closing brace or at `;`.
            let balance = code.matches('{').count() as i64 - code.matches('}').count() as i64;
            let one_line = code.trim_end().ends_with(';') && !code.contains('{');
            if one_line || (code.contains('{') && balance <= 0) {
                armed = false;
            } else if code.contains('{') {
                depth = Some(balance);
                armed = false;
            }
            continue;
        }
        if code.trim() == "#[cfg(test)]" {
            armed = true;
            continue;
        }
        kept.push((number + 1, code.to_string()));
    }
    kept
}

#[test]
fn the_core_holds_no_language_name_in_its_production_code() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    rust_files(&root.join("src"), &mut files);
    assert!(files.len() > 10, "the scan found {} files", files.len());
    let mut found = Vec::new();
    for file in files {
        let text = fs::read_to_string(&file).unwrap();
        for (number, line) in production_lines(&text) {
            let lower = line.to_lowercase();
            if LANGUAGE_WORDS.iter().any(|word| lower.contains(word)) {
                found.push(format!("{}:{number}: {}", file.display(), line.trim()));
            }
        }
    }
    assert!(
        found.is_empty(),
        "language names in the core:\n{}",
        found.join("\n")
    );
}

#[test]
fn the_core_depends_on_no_pack_crate() {
    let manifest =
        fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml")).unwrap();
    let dependencies = manifest
        .split("[dev-dependencies]")
        .next()
        .unwrap()
        .to_lowercase();
    for name in ["scippy-python", "scippy-typescript", "scippy-loci"] {
        assert!(!dependencies.contains(name), "{name}");
    }
}

#[test]
fn the_scanner_skips_test_items_and_comments() {
    let text = "\
fn kept() {}
// python in a comment
#[cfg(test)]
mod tests {
    fn gone() { let _ = \"python\"; }
}
#[cfg(test)]
use python_helper;
fn after() { let _ = \"typescript\"; }
";
    let lines: Vec<String> = production_lines(text).into_iter().map(|(_, l)| l).collect();
    assert!(lines.iter().any(|l| l.contains("fn kept")));
    assert!(lines.iter().all(|l| !l.contains("gone")));
    assert!(lines.iter().all(|l| !l.contains("python")));
    assert!(lines.iter().any(|l| l.contains("typescript")));
}

struct Model;

impl ProjectModel for Model {
    fn resolver_globals(&self, _: &ProjectRelativePath) -> Vec<(String, String)> {
        Vec::new()
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
            kind: SymbolKind::Variable,
            display_name: d.name.clone(),
        }
    }
}

/// A pack whose binding rules do not compile.
struct BadRules;

impl Analysis for BadRules {
    fn grammar(&self) -> tree_sitter::Language {
        tree_sitter_python::LANGUAGE.into()
    }
    fn binding_rules(&self) -> BindingRules {
        BindingRules {
            tsg_path: "bad.tsg",
            tsg_source: "(module) @m { this is not a rule",
            builtins: None,
            assets: Vec::new(),
            shadow_aware_similar_paths: false,
            prebuilt_assets: None,
        }
    }
    fn extraction_spec(&self) -> ExtractionSpec {
        ExtractionSpec {
            query: "(function_definition name: (identifier) @name) @definition.function",
        }
    }
    fn default_package(&self) -> Package {
        Package {
            manager: "test".into(),
            name: "bad".into(),
            version: ".".into(),
        }
    }
    fn project_model(
        &self,
        _: &Path,
        _: &[ProjectRelativePath],
        _: &[Manifest],
    ) -> Box<dyn ProjectModel> {
        Box::new(Model)
    }
}

impl LanguagePack for BadRules {
    fn id(&self) -> &'static str {
        "bad"
    }
    fn scip_language(&self) -> &'static str {
        "Bad"
    }
    fn accepts(&self, path: &Path) -> bool {
        path.extension().is_some_and(|e| e == "bad")
    }
    fn analysis(&self) -> Option<&dyn Analysis> {
        Some(self)
    }
}

#[test]
fn binding_rules_that_do_not_compile_fail_the_run_and_write_no_file() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("project");
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("a.bad"), "x = 1\n").unwrap();
    let output = dir.path().join("index.scip");
    let request = IndexRequest::new(&root, &output);
    let result = index_project(&request, &[&BadRules]);
    let message = match result {
        Ok(report) => panic!("the run passed: {:?}", report.diagnostics),
        Err(error) => error.to_string(),
    };
    assert!(message.contains("language pack bad"), "{message}");
    assert!(
        message.contains("binding rules do not compile"),
        "{message}"
    );
    assert!(!output.exists(), "the run wrote a file");
}
