# Scippy

Scippy is a Rust library and command-line tool for producing standard SCIP indexes.
The design documents are in the notes project, through the `.loci` link.

## Status

The Python pack (Gate 3) and the TypeScript pack (Gate 4) index real projects.
`scippy index PROJECT_ROOT --output index.scip` parses each Python and TypeScript file, resolves
names across files with the Stack Graphs engine, and writes definition, import, and reference
occurrences. A reference that the engine cannot resolve to one declaration gets no target.
`scippy languages` lists the built-in language packs.

## Workspace

- `crates/scippy-core` holds the language-neutral pipeline: the project walker with its
  input limits, the extractor, the binding engine adapter, the reconciler, symbol identity,
  byte-span positions, and the SCIP emitter. Only `index_project`, the request and report
  types, and the language-pack contract are public.
- `crates/scippy-cli` provides the `scippy` command.
- `languages/python` holds the Python pack: the declaration query, the import-root policy,
  and the symbol policy. It uses the binding rules of the fork.
- `languages/typescript` holds the TypeScript pack: the declaration query, two dialects (`.ts`
  and `.tsx`), the module and symbol policy, `package.json` and `tsconfig.json` support, and
  local fixes of the binding rules. The header of `languages/typescript/src/lib.rs` documents
  the symbol scheme, the supported layouts, and the known gaps.

The resolver is our fork, `Bullish-Intelligence/stack-graphs`. The root `Cargo.toml`
pins it to one commit for the four fork crates. To bump the pin, change the `rev` of the
four lines to the same commit and run the full verify.

## Input limits

The walker skips a file larger than 1 MiB (`--max-file-bytes`) with a `file-too-large` warning.
It stops to accept files when the accepted files would exceed 32 MiB in total
(`--max-total-bytes`). It then raises an `input-limit-reached` error diagnostic.
The graph build used about 1.3 GB for each MB of Python source.

## Symbols

Every symbol has the scheme `scippy`. Without a manifest, the Python pack uses the package
`python workspace`. The TypeScript pack takes the package from `package.json` at the project
root, and otherwise uses `npm workspace`. Pass `--package-name` and `--package-version` to
change them. A TypeScript module symbol is the project-relative path of the file with its
extension, for example `` `src/services.ts`/process(). ``.
A local variable inside a function gets a document-local symbol.
A parameter of a module-level function or of a method gets a global symbol with the parameter
descriptor, such as `pkg/mod/f().(name)`. A keyword argument in another document can name it.

## External symbols

A reference that ends in the builtins stub of the Python rules resolves to an external symbol.
The scheme is `scippy python python-stdlib typeshed-<12 hex digits> builtins/<descriptors>`,
for example `builtins/len().`, `builtins/str#`, and `builtins/str#join().`.
The version is the typeshed commit that the stub header names. The index lists each referenced
external symbol in `Index.external_symbols`, with kind and display name.
A definition in the project shadows the builtin of the same name.

The Python rules also ship stub modules, such as `typing`. A name of a stub module has the same
package and version, with the module in the descriptors: `typing/Any#`, `typing/`. A stub name
that aliases a builtin class, such as `typing.List`, has the symbol of the class, `builtins/list#`.
A project module of the same name, such as `typing.py`, wins over the stub.

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
- A reference at the span of a declaration of the same document names that declaration. The name
  in `self.x = v` is the definition of the field, not a second reference.
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
The tests run `scip lint`, `scip snapshot`, and `scip test` on the Python and TypeScript fixtures.
Set `SCIP_BIN_DIR` when `scip` is not on `PATH`.
Set `UPDATE_GOLDEN=1` to rewrite the golden snapshots in `languages/python/tests/golden` and
`languages/typescript/tests/golden`.
