# Scippy

Scippy is a Rust library and command-line tool for producing standard SCIP indexes.
The design documents are in the notes project, through the `.loci` link.

## Status

Gate 2 (the shared extraction and resolver adapter) is done for Python.
`scippy index PROJECT_ROOT --output index.scip` parses each Python file, resolves names
across files with the Stack Graphs engine, and writes definition, import, and reference
occurrences. A reference that the engine cannot resolve to one declaration gets no target.
The TypeScript pack only routes files. The run skips TypeScript files and says so.
`scippy languages` lists the built-in language packs.

## Workspace

- `crates/scippy-core` holds the language-neutral pipeline: the project walker with its
  input limits, the extractor, the binding engine adapter, the reconciler, symbol identity,
  byte-span positions, and the SCIP emitter. Only `index_project`, the request and report
  types, and the language-pack contract are public.
- `crates/scippy-cli` provides the `scippy` command.
- `languages/python` holds the Python pack: the declaration query, the import-root policy,
  and the symbol policy. It uses the binding rules of the fork.
- `languages/typescript` holds the TypeScript pack stub.

The resolver is our fork, `Bullish-Intelligence/stack-graphs`. The root `Cargo.toml`
pins it to one commit for the three fork crates. To bump the pin, change the `rev` of the
three lines to the same commit and run the full verify.

## Input limits

The walker skips a file larger than 1 MiB (`--max-file-bytes`) with a `file-too-large` warning.
It stops to accept files when the accepted files would exceed 32 MiB in total
(`--max-total-bytes`). It then raises an `input-limit-reached` error diagnostic.
The graph build used about 1.3 GB for each MB of Python source.

## Symbols

Every symbol has the scheme `scippy`. Without a manifest, the Python pack uses the package
`python workspace`. Pass `--package-name` and `--package-version` to change them.
A declaration inside a function gets a document-local symbol.

## Debugging an index

`--trace FILE` writes one line for each reference location with its outcome:
resolved, ambiguous, unresolved, or bounded. The file is not part of the index.

## Verify

Install the official `scip` CLI from https://github.com/scip-code/scip/releases.
Then run:

```sh
devenv shell -- scripts/verify.sh
```

The script runs `cargo fmt --check`, `cargo clippy -D warnings`, and `cargo test`.
The tests run `scip lint`, `scip snapshot`, and `scip test` on the Python fixtures.
Set `SCIP_BIN_DIR` when `scip` is not on `PATH`.
Set `UPDATE_GOLDEN=1` to rewrite the golden snapshots in `languages/python/tests/golden`.
