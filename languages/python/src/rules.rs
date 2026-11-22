//! The binding rules of the Python pack.
//!
//! The build script includes this file, so that it builds the asset graphs from the same rules.

use scippy_core::{BindingRules, Builtins, ExternalAsset};
use tree_sitter_stack_graphs_python::{STUB_MODULES, STUB_ROOT_PATH};

/// The rules without prebuilt assets.
pub fn binding_rules() -> BindingRules {
    BindingRules {
        tsg_path: tree_sitter_stack_graphs_python::STACK_GRAPHS_TSG_PATH,
        tsg_source: tree_sitter_stack_graphs_python::STACK_GRAPHS_TSG_SOURCE,
        builtins: Some(Builtins {
            path: tree_sitter_stack_graphs_python::STACK_GRAPHS_BUILTINS_PATH,
            source: tree_sitter_stack_graphs_python::STACK_GRAPHS_BUILTINS_SOURCE,
        }),
        assets: STUB_MODULES
            .iter()
            .map(|stub| ExternalAsset {
                name: stub.file_name(),
                path: stub.path,
                source: stub.source,
                globals: vec![("ROOT_PATH".to_string(), STUB_ROOT_PATH.to_string())],
            })
            .collect(),
        // The Python rules need it. The fork sets the same flag in its language configuration.
        shadow_aware_similar_paths: true,
        // No cap. A cap of 1 changed no result on the evaluation corpora and saved no time.
        max_scope_depth: None,
        prebuilt_assets: None,
    }
}
