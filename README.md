# Scippy

Scippy is a Rust library and command-line tool for producing standard SCIP indexes.
The current repository contains the workspace scaffold. Indexing behavior is not implemented.

## Workspace

- `crates/scippy-core` will provide the language-neutral indexing API.
- `crates/scippy-cli` will provide the `scippy` command.
- `languages/python` will hold Python-specific parsing and binding rules.
- `languages/typescript` will hold TypeScript-specific parsing and binding rules.

The dependency viability spike is the first implementation gate. Add Tree-sitter,
Stack Graphs, and SCIP dependencies after that spike confirms the required APIs.
