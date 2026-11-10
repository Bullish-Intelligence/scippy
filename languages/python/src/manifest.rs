//! The `pyproject.toml` of the project root and of each shard.
//!
//! The pack reads four things from a manifest:
//! - the project name, normalized as PEP 503 says, to match it to a dependency name;
//! - the version, when the manifest gives it as a string;
//! - the declared dependencies: `[project] dependencies`, the optional dependencies, the
//!   dependency groups, and the Poetry dependency tables;
//! - the package folders: the folders and module files that the wheel installs.
//!
//! The package folders come from the first source that names any:
//! `[tool.hatch.build.targets.wheel] packages`, `[tool.uv.build-backend]`,
//! `[tool.poetry] packages`, and `[tool.setuptools.packages.find] where`.
//! Without them, the pack looks for the import name of the project in `src/` and then in the
//! manifest folder. The import name is the normalized name with `_` for `-`.
//! Only a package folder that holds a file of the pack counts.

use std::collections::BTreeSet;

use toml::{Table, Value};

/// What the pack reads from one manifest. Every path is project-relative.
#[derive(Debug, Default)]
pub(crate) struct Project {
    /// The folder of the manifest. The project root is `""`.
    pub folder: String,
    /// The project name as the manifest writes it.
    pub raw_name: Option<String>,
    /// The project name, normalized.
    pub name: Option<String>,
    pub version: Option<String>,
    /// The normalized names of the declared dependencies, sorted.
    pub dependencies: BTreeSet<String>,
    /// The folders and module files that the wheel installs.
    pub packages: Vec<String>,
}

/// Normalize a distribution name as PEP 503 says: lower case, and one `-` for each run of
/// `-`, `_`, and `.`.
pub(crate) fn normalize(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for c in name.chars() {
        if matches!(c, '-' | '_' | '.') {
            if !out.ends_with('-') {
                out.push('-');
            }
        } else {
            out.extend(c.to_lowercase());
        }
    }
    out
}

/// The distribution name at the start of a requirement such as `pydantic>=2; python_version > "3"`.
fn requirement_name(requirement: &str) -> Option<String> {
    let name: String = requirement
        .trim()
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        .collect();
    (!name.is_empty()).then(|| normalize(&name))
}

fn table<'a>(root: &'a Table, path: &[&str]) -> Option<&'a Table> {
    let mut at = root;
    for key in path {
        at = at.get(*key)?.as_table()?;
    }
    Some(at)
}

fn strings(value: Option<&Value>) -> Vec<String> {
    match value {
        Some(Value::String(s)) => vec![s.clone()],
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|v| v.as_str().map(String::from))
            .collect(),
        _ => Vec::new(),
    }
}

/// Join a path of the manifest to its folder. Drop `./` and a trailing `/`.
fn join(folder: &str, path: &str) -> String {
    let path = path.trim_start_matches("./").trim_end_matches('/');
    match (folder.is_empty(), path.is_empty() || path == ".") {
        (_, true) => folder.to_string(),
        (true, false) => path.to_string(),
        (false, false) => format!("{folder}/{path}"),
    }
}

impl Project {
    /// Read a manifest. `files` lists the files of the pack. Return `None` when the text is
    /// not TOML.
    pub fn read(folder: &str, text: &str, files: &[&str]) -> Option<Self> {
        let root: Table = text.parse().ok()?;
        let project = table(&root, &["project"]);
        let poetry = table(&root, &["tool", "poetry"]);
        let raw_name = project
            .and_then(|p| p.get("name"))
            .or_else(|| poetry.and_then(|p| p.get("name")))
            .and_then(Value::as_str)
            .map(String::from);
        let version = project
            .and_then(|p| p.get("version"))
            .or_else(|| poetry.and_then(|p| p.get("version")))
            .and_then(Value::as_str)
            .map(String::from);
        let mut out = Project {
            folder: folder.to_string(),
            name: raw_name.as_deref().map(normalize),
            raw_name,
            version,
            ..Project::default()
        };
        out.dependencies = dependencies(&root);
        if let Some(name) = &out.name {
            out.dependencies.remove(name);
        }
        out.packages = packages(&root, folder, out.name.as_deref(), files);
        Some(out)
    }

    /// Say whether the path is a package folder of the project or lies below one.
    /// Return the package folder.
    pub fn package_of<'a>(&'a self, path: &str) -> Option<&'a str> {
        self.packages
            .iter()
            .filter(|p| {
                path == p.as_str()
                    || path
                        .strip_prefix(p.as_str())
                        .is_some_and(|rest| rest.starts_with('/'))
            })
            .max_by_key(|p| p.len())
            .map(String::as_str)
    }
}

fn dependencies(root: &Table) -> BTreeSet<String> {
    let mut requirements: Vec<String> = Vec::new();
    if let Some(project) = table(root, &["project"]) {
        requirements.extend(strings(project.get("dependencies")));
        if let Some(optional) = project
            .get("optional-dependencies")
            .and_then(Value::as_table)
        {
            for group in optional.values() {
                requirements.extend(strings(Some(group)));
            }
        }
    }
    if let Some(groups) = table(root, &["dependency-groups"]) {
        for group in groups.values() {
            requirements.extend(strings(Some(group)));
        }
    }
    requirements.extend(strings(
        table(root, &["tool", "uv"]).and_then(|t| t.get("dev-dependencies")),
    ));
    let mut names: BTreeSet<String> = requirements
        .iter()
        .filter_map(|r| requirement_name(r))
        .collect();
    // Poetry names a dependency by its key.
    let mut poetry_tables: Vec<&Table> = Vec::new();
    for key in ["dependencies", "dev-dependencies"] {
        poetry_tables.extend(table(root, &["tool", "poetry", key]));
    }
    if let Some(groups) = table(root, &["tool", "poetry", "group"]) {
        for group in groups.values().filter_map(Value::as_table) {
            poetry_tables.extend(group.get("dependencies").and_then(Value::as_table));
        }
    }
    for t in poetry_tables {
        names.extend(t.keys().filter(|k| *k != "python").map(|k| normalize(k)));
    }
    names
}

/// The package folders and module files of the wheel, as project-relative paths that exist.
fn packages(root: &Table, folder: &str, name: Option<&str>, files: &[&str]) -> Vec<String> {
    let exists = |path: &str| {
        files.iter().any(|f| {
            *f == path
                || f.strip_prefix(path)
                    .is_some_and(|rest| rest.starts_with('/'))
        })
    };
    let import_name = name.map(|n| n.replace('-', "_"));
    let mut declared: Vec<String> = Vec::new();
    // Hatch lists the folders: `packages = ["src/pkg"]`.
    declared.extend(strings(
        table(root, &["tool", "hatch", "build", "targets", "wheel"])
            .and_then(|t| t.get("packages")),
    ));
    // The uv build backend names a module and a module root. The root defaults to `src`.
    if declared.is_empty() {
        let backend = root
            .get("build-system")
            .and_then(|b| b.get("build-backend"))
            .and_then(Value::as_str);
        let uv = table(root, &["tool", "uv", "build-backend"]);
        if uv.is_some() || backend == Some("uv_build") {
            let module_root = uv
                .and_then(|t| t.get("module-root"))
                .and_then(Value::as_str)
                .unwrap_or("src");
            let mut modules = strings(uv.and_then(|t| t.get("module-name")));
            if modules.is_empty() {
                modules.extend(import_name.clone());
            }
            declared.extend(
                modules
                    .iter()
                    .map(|m| join(module_root, &m.replace('.', "/"))),
            );
        }
    }
    // Poetry lists `{ include = "pkg", from = "src" }`.
    if declared.is_empty() {
        if let Some(items) = table(root, &["tool", "poetry"])
            .and_then(|t| t.get("packages"))
            .and_then(Value::as_array)
        {
            for item in items.iter().filter_map(Value::as_table) {
                let Some(include) = item.get("include").and_then(Value::as_str) else {
                    continue;
                };
                let from = item.get("from").and_then(Value::as_str).unwrap_or("");
                declared.push(join(from, include));
            }
        }
    }
    let mut out: Vec<String> = declared
        .iter()
        .map(|p| join(folder, p))
        .filter(|p| !p.contains('*') && exists(p))
        .collect();
    // Setuptools finds the packages below each `where` folder.
    if out.is_empty() {
        for place in strings(
            table(root, &["tool", "setuptools", "packages", "find"]).and_then(|t| t.get("where")),
        ) {
            let base = join(folder, &place);
            let top: BTreeSet<String> = files
                .iter()
                .filter_map(|f| {
                    let rest = if base.is_empty() {
                        Some(*f)
                    } else {
                        f.strip_prefix(base.as_str())?.strip_prefix('/')
                    };
                    let (first, below) = rest?.split_once('/')?;
                    (below == "__init__.py").then(|| join(&base, first))
                })
                .collect();
            out.extend(top);
        }
    }
    // The usual layouts: `src/<import name>` and `<import name>`, as a folder or a module.
    if out.is_empty() {
        if let Some(import_name) = &import_name {
            let candidates = [
                format!("src/{import_name}"),
                format!("src/{import_name}.py"),
                import_name.clone(),
                format!("{import_name}.py"),
            ];
            if let Some(found) = candidates
                .iter()
                .map(|c| join(folder, c))
                .find(|p| exists(p))
            {
                out.push(found);
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_follow_pep_503() {
        assert_eq!(normalize("Lib_Pkg.core"), "lib-pkg-core");
        assert_eq!(normalize("a--_b"), "a-b");
        assert_eq!(
            requirement_name("Loci_Core[cli]>=0.6; python_version>'3'").as_deref(),
            Some("loci-core")
        );
        assert_eq!(requirement_name(" ").as_deref(), None);
    }

    #[test]
    fn a_manifest_gives_its_dependencies_and_its_package_folders() {
        let text = r#"
            [project]
            name = "My_App"
            version = "1.2"
            dependencies = ["Lib.Pkg>=1", "my-app[extra]"]
            [project.optional-dependencies]
            cli = ["typer"]
            [dependency-groups]
            dev = ["pytest", { include-group = "cli" }]
            [tool.hatch.build.targets.wheel]
            packages = ["src/my_app", "src/missing"]
        "#;
        let files = ["app/src/my_app/__init__.py", "app/tests/test_a.py"];
        let p = Project::read("app", text, &files).unwrap();
        assert_eq!(p.name.as_deref(), Some("my-app"));
        assert_eq!(p.version.as_deref(), Some("1.2"));
        let deps: Vec<&str> = p.dependencies.iter().map(String::as_str).collect();
        assert_eq!(deps, ["lib-pkg", "pytest", "typer"]);
        assert_eq!(p.packages, ["app/src/my_app"]);
        assert_eq!(
            p.package_of("app/src/my_app/x/y.py"),
            Some("app/src/my_app")
        );
        assert_eq!(p.package_of("app/src/my_app_extra/y.py"), None);
        assert_eq!(p.package_of("app/tests/test_a.py"), None);
    }

    #[test]
    fn without_a_declared_folder_the_import_name_finds_the_package() {
        let files = ["src/demo/__init__.py", "flat/x.py"];
        let p = Project::read("", "[project]\nname = \"Demo\"\n", &files).unwrap();
        assert_eq!(p.packages, ["src/demo"]);
        let files = ["tool.py"];
        let p = Project::read("", "[project]\nname = \"tool\"\n", &files).unwrap();
        assert_eq!(p.packages, ["tool.py"]);
        let files = ["lib/pkg/__init__.py", "lib/pkg/m.py", "lib/other.py"];
        let text = "[project]\nname = \"x\"\n[tool.setuptools.packages.find]\nwhere = [\"lib\"]\n";
        let p = Project::read("", text, &files).unwrap();
        assert_eq!(p.packages, ["lib/pkg"]);
        assert!(Project::read("", "not toml [", &files).is_none());
    }
}
