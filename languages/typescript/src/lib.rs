//! TypeScript language pack.
//!
//! Gate 1 routes files only. Parsing and binding rules come with Gate 4.

use std::path::Path;

use scippy_core::LanguagePack;

pub struct TypeScriptPack;

impl LanguagePack for TypeScriptPack {
    fn id(&self) -> &'static str {
        "typescript"
    }

    fn scip_language(&self) -> &'static str {
        "TypeScript"
    }

    fn accepts(&self, path: &Path) -> bool {
        path.extension().is_some_and(|e| e == "ts" || e == "tsx")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_typescript_sources_only() {
        assert!(TypeScriptPack.accepts(Path::new("src/app.ts")));
        assert!(TypeScriptPack.accepts(Path::new("src/App.tsx")));
        assert!(!TypeScriptPack.accepts(Path::new("src/app.js")));
        assert!(!TypeScriptPack.accepts(Path::new("app.py")));
    }
}
