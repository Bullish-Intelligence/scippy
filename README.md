# Scippy

Scippy is a Rust library and command-line tool for producing standard SCIP indexes.
The design documents are in the notes project, through the `.loci` link.

## Status

Gate 1 (the SCIP producer) is done. Scippy can write a valid `index.scip` from facts.
No stage yet extracts declarations or resolves references.
`scippy index PROJECT_ROOT --output index.scip` therefore writes one document for each
Python and TypeScript file, with no occurrences.
`scippy languages` lists the built-in language packs.

## Workspace

- `crates/scippy-core` holds the language-neutral indexing API: the project walker,
  symbol identity, byte-span positions, and the SCIP emitter.
- `crates/scippy-cli` provides the `scippy` command.
- `languages/python` holds Python-specific parsing and binding rules.
- `languages/typescript` holds TypeScript-specific parsing and binding rules.

Gate 0 (the dependency viability spike) must finish before Scippy adds Tree-sitter
and Stack Graphs dependencies.

## Verify

Install the official `scip` CLI from https://github.com/scip-code/scip/releases.
Then run:

```sh
devenv shell -- scripts/verify.sh
```

The script runs `cargo fmt --check`, `cargo clippy -D warnings`, and `cargo test`.
The tests run `scip lint`, `scip snapshot`, and `scip test`.
Set `SCIP_BIN_DIR` when `scip` is not on `PATH`.
Set `UPDATE_GOLDEN=1` to rewrite the golden files in `crates/scippy-core/tests/golden`.
