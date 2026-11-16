//! The static registry of built-in language packs.

use scippy_core::LanguagePack;
use scippy_loci::LociPack;
use scippy_python::PythonPack;
use scippy_typescript::TypeScriptPack;

/// The built-in packs. Indexing and discovery both use this list.
pub fn registry() -> [&'static dyn LanguagePack; 3] {
    [&PythonPack, &TypeScriptPack, &LociPack]
}
