# Scippy

Scippy is a Rust library and command-line tool for producing standard SCIP indexes.
The design documents are in the notes project, through the `.loci` link.

## Status

The Python pack (Gate 3) and the TypeScript pack (Gate 4) index real projects.
`scippy index PROJECT_ROOT --output index.scip` parses each Python and TypeScript file, resolves
names across files with the Stack Graphs engine, and writes definition, import, and reference
occurrences. A reference that the engine cannot resolve to one declaration gets no target.
`scippy languages` lists the built-in language packs.

The Loci pack (Gate 5) indexes a notes vault: a project root that has `.loci/vault.toml`.
Without that file, the pack is off and `.md` files belong to no pack. The pack follows
`loci-core`: the same scanner, the same resolution order, and the same states. A tie is
`ambiguous` and gets no occurrence. The test oracle is `loci --json graph/relations`.
The pack reads `[[root]]`, `roots_file`, and `[policy] ignore_file`, as `loci-core` does. A root is a
code repository. Its Python files go to the Python pack and its notes to the Loci pack. A path that
the ignore list or `discovery.exclude` hides is hidden from every pack. A note that is not UTF-8 is
skipped with a warning, where `loci-core` reads it lossy. A link with a resolved anchor has two
occurrences: one over the target part, for the note, and one over the anchor part.

Each root is a repository of its own. The core resolves the code of each root with a binding
engine of its own, and frees the engine after the root. A root sees the code of another root
only through its manifest. The roots run in parallel (see "Parallel roots").

## Manifests

The Python pack reads the `pyproject.toml` of the project root and of each root.

- The manifest of the project root names the package of the symbols: the project name and the
  version.
- The pack finds the package folders of a manifest: the folders that the wheel installs. It reads
  Hatch, the uv build backend, Poetry, and setuptools. Without them, it looks for the import name
  in `src/` and then in the root folder.
- A package folder such as `src/pkg` makes `src` the import root of every file below it. A folder
  without `__init__.py` is then a namespace package. A file outside every package folder keeps the
  `__init__.py` rule. The symbols do not change.
- The pack reads the declared dependencies: `[project] dependencies`, the optional dependencies,
  the dependency groups, and the Poetry tables. It normalizes each name as Python Enhancement
  Proposal (PEP) 503 says.
- A dependency whose name is the project name of exactly one other root makes that root a
  dependency. The dependencies of a dependency count too.

The core loads the package folders of each dependency into the binding engine of the root. The
engine reads them to resolve references only. A dependency adds no document and no occurrence to
the root. Tests and scripts of a dependency stay out. Without a manifest, nothing changes.

## Workspace

- `crates/scippy-core` holds the language-neutral pipeline: the project walker with its
  input limits, the extractor, the binding engine adapter, the reconciler, symbol identity,
  byte-span positions, and the SCIP emitter. Only `index_project`, the request and report
  types, and the language-pack contract are public.
- `crates/scippy-cli` provides the `scippy` command.
- `languages/python` holds the Python pack: the declaration query, the manifest reader, the
  import-root policy, and the symbol policy. It uses the binding rules of the fork.
- `languages/typescript` holds the TypeScript pack: the declaration query, two dialects (`.ts`
  and `.tsx`), the module and symbol policy, `package.json` and `tsconfig.json` support, and
  local fixes of the binding rules. The header of `languages/typescript/src/lib.rs` documents
  the symbol scheme, the supported layouts, and the known gaps.
- `languages/loci` holds the Loci pack. It is a native engine: it reads its files and resolves
  its links itself, and the core joins, names, and writes the result. The header of
  `languages/loci/src/lib.rs` documents the selection rule and the symbol scheme.

The resolver of the Python and TypeScript packs is our fork, `Bullish-Intelligence/stack-graphs`. The root `Cargo.toml`
pins it to one commit for the four fork crates. To bump the pin, change the `rev` of the
four lines to the same commit and run the full verify.
The pin is `9a584b8`. That commit speeds up the stitcher and adds the option to turn off the debug
attributes (see "Performance" below). Commit `91450c8` resolves the names in `match` patterns. Commit `013c27c` added
the synthesized `__init__` of model classes and `TypedDict` classes (see below). Commit `be7ece9` added type parameters, type aliases, the walrus, and relative
imports with any number of dots. Commit `55f33f4` added the synthesized `__init__` of dataclasses
and `NamedTuple` classes, the keyword rule for star parameters, and the `global` and `nonlocal`
rules. Commit `e5cba0a` added shadow-aware similar
path detection. The detector keeps
the first of two similar paths. Without the mode, that path can be one that another path shadows,
and a name loses a binding. The Python pack turns the mode on in its binding rules. The resolver
uses it when it stores partial paths and when it resolves references. The TypeScript pack leaves
it off.

### Keyword arguments of classes

A keyword argument in `C(name=v)` names a field of `C` in these cases. The rules read the text of
the class definition. They do not check that a name resolves to the library.

- `C` is a dataclass: the decorator is `dataclass`, `pydantic_dataclass`, or one of `attrs.define`,
  `attrs.frozen`, `attr.s`.
- `C` is a `NamedTuple` or a `TypedDict` (with `total=False` too).
- `C` is a model class: it derives from `pydantic.BaseModel`, `pydantic_settings.BaseSettings`, or
  `sqlmodel.SQLModel`. It also counts when `C` derives from a class of the project that is a model
  class, or from a class that `@dataclass_transform` marks.
- The subscript form `C[T](name=v)` and the generic base `class B(A[T])` reach `C` and `A`.

A model class skips a `ClassVar`, a private field `_x`, and a `PrivateAttr`. A field takes
its keyword from the field name. The alias in `Field(alias=...)` is not a keyword here. A written
`__init__` wins over the fields. The rules do not follow a metaclass that `dataclass_transform`
marks, or a decorator of the project that it marks.

### Element types of containers

A written container annotation gives the type of its elements. The rules read the name of the
annotation only: `list`, `Sequence`, `Iterable`, `Iterator`, `set`, `frozenset`, `deque`,
`tuple[T, ...]`, and their `typing` spellings, also inside `Optional` or a union with `None`.

- `for x in xs` and `[x for x in xs]` give `x` the element type.
- `xs[i]` gives the element type. `d[k]` on `dict`, `Mapping`, or `defaultdict` gives the value type.
- `d.keys()`, `d.values()`, `d.items()`, `d.get(k)`, `d.pop(k)`, and `d.setdefault(k, v)` give the
  key or the value type.
- A tuple pattern, such as `for k, v in ...`, reads `tuple[A, B]`, `d.items()`, `enumerate(xs)`, and
  `zip(xs, ys)` by position. `sorted`, `reversed`, `list`, `set`, `tuple`, and `iter` keep the
  element type.
- `*args: T` has elements of type `T`. `**kwargs: T` has values of type `T`.

A container class of the project does not give its element type. Neither does a dict subclass
with type arguments in its base class.

## Input limits

The walker skips a file larger than 1 MiB (`--max-file-bytes`) with a `file-too-large` warning.
It stops to accept files when the accepted files would exceed 32 MiB in total
(`--max-total-bytes`). It then raises an `input-limit-reached` error diagnostic.
The graph build used up to 1.3 GB for each MB of Python source.
The Loci pack needs far less memory. When a vault is active, the default per-file limit is the
largest `note_max_bytes` of the vault and its roots, and the default total limit is 4 GiB.
A flag still wins.

## Workspace run

The first real workspace is the `~/Notes` vault with 57 code roots: 42,340 files, 12,525
indexed documents, and about 36 MB of Python. The Loci relations agree with `loci-core` on all
35,333 relations.

| Run | Peak memory | Wall time | Occurrences |
| --- | --- | --- | --- |
| One binding engine for all roots | 19.8 GB | 16:56 | 1,363,480 |
| One binding engine for each root | 5.7 GB | 18:51 | 1,362,852 |
| Each root with its declared dependencies | 5.7 GB | 19:58 | 1,366,048 |

The two indexes have the same documents, declarations, and external symbols. 380 documents
differ. Without shards, 2,957 references resolved to code in another root, and 24 more reached a
builtin through such code. With shards, those references are unresolved. 2,353 references that
had a second candidate in another root were ambiguous. They now resolve to their own root.
The run is slower because each root compiles the rules and loads the builtins again.

The manifests bring back 998 of the 2,957 references, each with the target of the run without
shards. The other references go to roots that the manifest does not declare. `basix` and `minix`
import `pynix` (1,722 references), and their `devenv.nix` adds it to the environment. `argentic`
imports `eventic` (151). `argentic` and `devenv-py` declare `pydantree-sitter`, and the root names
itself `pydantree` (49). The package folders resolve about 2,100 more references in their own root,
mostly through namespace packages. 26 references in `semantic-views` now reach the work limit,
and 4 in `argentic` are ambiguous.

## Symbols

Every symbol has the scheme `scippy`. Without a manifest, the Python pack uses the package
`python workspace`. With a `pyproject.toml` at the project root, it uses the project name and the
version. The TypeScript pack takes the package from `package.json` at the project
root, and otherwise uses `npm workspace`. Pass `--package-name` and `--package-version` to
change them. A TypeScript module symbol is the project-relative path of the file with its
extension, for example `` `src/services.ts`/process(). ``.
A local variable inside a function gets a document-local symbol.
A parameter of a module-level function or of a method gets a global symbol with the parameter
descriptor, such as `pkg/mod/f().(name)`. A keyword argument in another document can name it.
A keyword argument never names `*args` or `**kwargs`.
A `@dataclass` class and a `NamedTuple` class get a synthesized `__init__`. A keyword argument of
a call of the class names the field, such as `pkg/mod/Point#x.`. A subclass with no `__init__`
uses the fields of its base classes. A class that writes its own `__init__` uses only that.
The rules read the decorator by its text, `dataclass` or `dataclasses.dataclass`. They do not
check that the name comes from the `dataclasses` module.
A name that `global` or `nonlocal` declares resolves to the binding of the module or of the
enclosing function. The assignment in the function still makes a local definition, but no
reference reaches it.
A type parameter, as in `class C[T: Base]` or `def f[T]()`, has its own scope. It covers the
whole definition and wins over a module variable of the same name. It gets a document-local
symbol. A `type Name = value` statement defines `Name`.
A walrus `name := value` binds `name` in the enclosing function or module, also inside a comprehension.
A `match` statement binds its captures in the enclosing function or module: `case x:`,
`case [a, *rest]:`, `case {"k": v, **kw}:`, and the name of `as`. The wildcard `_` binds nothing.
A name that a `case` binds stays visible after the `match` statement.
A class pattern `case Point(x=0):` references the class. Its keyword name `x` references the
member `x` of the class. A dotted name, as in `case Color.RED:`, is a reference.
A relative import takes any number of dots. An import that climbs above the project root does
not resolve.

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
- It drops the end of a member reference when the path pops the name of the reference at an
  assignment target that no declaration claims, as in `item.kind = value` with an unknown type
  of `item`. The path then follows the assigned value. The reference has no target.
- A search that reaches the work limit keeps its result when a plain name has a nearest binding
  in the same document and no other nearest end. A rebind such as `left, right = f(left, right)`
  reaches the limit often. A member reference, a keyword name, and other limits stay bounded.
- It asks the language pack whether a reference can name each joined declaration, and drops
  the ends that the pack refuses. The Python pack refuses an instance field for a plain name,
  because Python needs `self.x`. It also refuses a name that a class body binds, for a plain name
  outside that body, because a method does not see class scope. A reference whose every end the
  pack refuses is unresolved with the reason `not-targetable`.
- A reference at the span of a declaration of the same document names that declaration. The name
  in `self.x = v` is the definition of the field, not a second reference.
- A reference with no target gets no occurrence.

### Narrowing by `isinstance`

The Python rules narrow the receiver of `x.attr` inside a block that `isinstance(x, C)` guards.
The receiver then has an instance of `C`, so `x.attr` reaches the members of `C` and its bases.
The rules read the written guard only. They infer no type.

- The body of an `if` or an `elif` is narrowed. So is the right side of `and`.
- The rest of a block is narrowed after `assert isinstance(x, C)`, and after
  `if not isinstance(x, C):` when its body ends with `return`, `raise`, `continue`, or `break`.
- `x` is a name or an attribute of a name. `C` is a name, a dotted name, or a tuple of them.
  A tuple gives one candidate for each class.
- The narrowing stays inside the guarded statements. An inner guard shadows an outer guard.
  The declared type still resolves a member that `C` lacks.

## Debugging an index

`--trace FILE` writes one line for each reference location with its outcome:
resolved, ambiguous, unresolved, or bounded. The file is not part of the index.

## Performance

The index keeps its output when it gets faster. Three changes of the fork and one of Scippy
make an index about 30 to 40 percent cheaper in CPU time and up to 30 percent smaller in peak
memory. The stitcher rejects a join early when the first symbols clash. The cycle detector uses a
Bloom filter. Scippy turns off the debug attributes of the fork with
`StackGraphLanguage::set_debug_attributes(false)`. The pipeline compiles the TSG rules once and
shares them across shards. Parallel shards cut the wall time of the workspace to a third.
`scripts/oracle/RESULTS.md` has the numbers. To check a speed change,
index the oracle corpora with the old and the new build, and compare the `.scip` and `--trace`
files with `cmp`.

## Parallel roots

The core indexes the roots (shards) on worker threads. Each worker owns the binding engines that
it builds, so no engine crosses a thread. Identities, reconciliation, and output stay global. The
core merges the results in the order of the shards. The `.scip` file and the `--trace` file do not
depend on the number of workers.

- `--jobs N` sets the number of workers. The default is the number of logical cores. `--jobs 1` runs the
  shards one at a time, as before.
- `--memory-budget SIZE` (for example `8G`) sets the memory that the running shards may use
  together. The default is 50G.
- The core estimates the peak of a shard from the source bytes that its engine loads: the shard
  and the code that it imports. The estimate is 300 MiB plus 3,000 bytes for each byte of loaded
  source, and the second part stops at 10 GiB. The largest measured peak is 8.08 GB.
- The scheduler starts the largest shards first. It starts a shard only when the estimates of the
  running shards and of this shard stay under the budget. A shard over the budget runs alone.
- A project with one shard, or no shards, runs on the calling thread.

Set `SCIPPY_SHARD_LOG=1` to print the estimate, the start resident memory, and the peak resident
memory of each shard. A sampling thread reads `VmRSS` every 10 ms, so the log does not change the
high-water mark that `time -v` reports. The figures are for the whole process: they are exact for
one shard only with `--jobs 1`. With more jobs, they include the other running shards. `scripts/oracle/RESULTS.md` has the measurements.

## Accuracy oracle

`scripts/oracle` compares a Scippy index with a `scip-python` index of the same corpus.
`scip-python` is the reference only. Scippy never uses a type checker to resolve names.
The corpora are mypy and rich. `scripts/oracle/RESULTS.md` records each run.

1. Put the corpora, `scip-python`, and `env-empty.json` in `ORACLE_DIR`.
   The default is `~/.local/state/scippy/oracle`. `run_all.sh` lists the setup.
2. Build Scippy in release mode. Set `SCIPPY_BIN` to the binary.
3. Run `scripts/oracle/run_all.sh OUTDIR`. It indexes one corpus at a time.
4. To compare existing indexes again, run `scripts/oracle/recompute.sh CORPUS OUTDIR`.
5. To count the references inside `case` clauses, run
   `python3 -I scripts/oracle/caseblocks.py ROOT SCIPPY.scip SCIP_PYTHON.scip`.

The five eval corpora (`agentman`, `eventic`, `argentic`, `inferference`, `pydantic-ai`) sit in
another `ORACLE_DIR`. Run them with the `mkenv.sh` environment: set `ORACLE_VENV=1` for `scip-python`.

`compare.py` reads the stubs of the fork. Set `STACK_GRAPHS_DIR` to a checkout at the pin.
`metrics.py` prints precision, wrong targets, raw recall, and recall without design gaps.
Raw recall counts a rebind as matched when the Scippy definition has the definition location
of the `scip-python` target. A keyword argument that resolves to `*args` or `**kwargs` is a wrong
target. The design gaps are the rebinds without that credit, keyword names of plain functions,
and skipped files.

By default `scip-python` runs with an empty environment, so it cannot see third-party packages.
It then emits no keyword references to the fields of Pydantic models. To measure those, run it
with a per-corpus environment:

1. Run `scripts/oracle/mkenv.sh CORPUS`. It uses `uv` to install wheels of `pydantic` into
   `ORACLE_ENVS/CORPUS` (default `ORACLE_DIR/envs`). It adds `pydantic-settings`, `sqlmodel`, and
   `typing_extensions` when the corpus imports them. It never installs or runs the corpus.
2. Set `ORACLE_VENV=1` and run `scripts/oracle/run_tools.sh CORPUS CORPUS scip-python OUTDIR`.
3. Compare as before with `recompute.sh`. Use a new OUTDIR, and copy `scippy.scip` and
   `scippy.trace` into it, so the empty-environment numbers stay.

Targets in installed packages are external. `compare.py` counts only targets that have a
definition in the corpus, so recall and precision keep their meaning.

## Verify

Install the official `scip` CLI from https://github.com/scip-code/scip/releases.
Then run:

```sh
devenv shell -- scripts/verify.sh
```

The script runs `cargo fmt --check`, `cargo clippy -D warnings`, and `cargo test`.
The tests run `scip lint`, `scip snapshot`, and `scip test` on the Python, TypeScript, and Loci fixtures.
The Loci expectations come from `loci-core`. To make them again, run `scripts/loci-oracle.sh`.
Set `SCIP_BIN_DIR` when `scip` is not on `PATH`.
Set `UPDATE_GOLDEN=1` to rewrite the golden snapshots in `languages/python/tests/golden` and
`languages/typescript/tests/golden`, and `languages/loci/tests/golden`.
