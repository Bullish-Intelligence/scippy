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

## External symbols

A reference that ends in the builtins stub of the Python rules resolves to an external symbol.
The scheme is `scippy python python-stdlib typeshed-<12 hex digits> builtins/<descriptors>`,
for example `builtins/len().`, `builtins/str#`, and `builtins/str#join().`.
The version is the typeshed commit that the stub header names. The index lists each referenced
external symbol in `Index.external_symbols`, with kind and display name.
A definition in the project shadows the builtin of the same name.

## Resolution policy

The reconciler turns the engine paths of one reference into one target, or none.

- It drops a path that another path shadows, and it merges the reference nodes at one span.
- It ignores an end that joins no declaration, such as an import alias or a module.
- It prefers a definition in the project over a definition in an external asset.
- It counts distinct symbols. One symbol is Resolved. Two or more are Ambiguous.
- It prefers the nearest binding. The engine follows values, so a name such as `self` reaches
  its parameter and then the class that the value comes from. The reconciler drops an end
  when its path passes the definition of another symbol. The language pack marks member
  references with the `@member` capture. For `a.x`, only a definition that is an end of the
  same reference counts, because every path passes the definition of `a`.
- It asks the language pack whether a reference can name each joined declaration, and drops
  the ends that the pack refuses. The Python pack refuses an instance field for a plain name,
  because Python needs `self.x`. It also refuses a name that a class body binds, for a plain name
  outside that body, because a method does not see class scope. A reference whose every end the
  pack refuses is unresolved with the reason `not-targetable`.
- A reference with no target gets no occurrence.

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
