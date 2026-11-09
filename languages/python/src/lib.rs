//! Python language pack.
//!
//! Gate 1 routes files only. Parsing and binding rules come with Gate 3.

use std::path::Path;

use scippy_core::LanguagePack;

pub struct PythonPack;

impl LanguagePack for PythonPack {
    fn id(&self) -> &'static str {
        "python"
    }

    fn scip_language(&self) -> &'static str {
        "Python"
    }

    fn accepts(&self, path: &Path) -> bool {
        path.extension().is_some_and(|e| e == "py")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_python_sources_only() {
        assert!(PythonPack.accepts(Path::new("src/app.py")));
        assert!(!PythonPack.accepts(Path::new("src/app.pyc")));
        assert!(!PythonPack.accepts(Path::new("app.ts")));
        assert!(!PythonPack.accepts(Path::new("py")));
    }
}
