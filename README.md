# Scippy

Scippy is a Rust library and command-line tool for producing standard SCIP indexes.
The design documents are in the notes project, through the `.loci` link.

## Status

The Python pack (Gate 3) indexes real projects. The TypeScript pack (Gate 4) is a calibration
guard (ADR-011): it runs in the tests, and it is not supported.
`scippy index PROJECT_ROOT --output index.scip` parses each Python and TypeScript file, resolves
names across files with the Stack Graphs engine, and writes definition, import, and reference
occurrences. A reference that the engine cannot resolve to one declaration gets no target, with
one exception: a union. When the candidates of an ambiguous reference are members of at most four
classes that have no inheritance relation (`e: A | B` then `e.items`, or `isinstance(x, (A, B))`),
the index has one occurrence for each candidate, all at one range. SCIP allows several
occurrences at one range.
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

## Support matrix (v0.1)

This table says only what the tests cover.

| Item | Status |
|---|---|
| Python | Supported. The oracle counts 99.4% (mypy) and 99.96% (rich) of shared positions as correct (`scripts/oracle/RESULTS.md`). |
| Loci Markdown | Supported. The pack follows `loci-core`. The saved comparison on a real vault has 0 disagreements. It was not rerun after commit `04caacd`. |
| TypeScript | Experimental guard only (ADR-011). The tests keep it from regressing. The project makes no promise about its output. |
| Metalayer and identity store | Experimental. No stability promise for entity IDs or the schema. |
| Output | SCIP. `scip lint`, `scip snapshot`, and `scip test` check it in `opt:slow`. |
| Graph cache | On by default. It is disposable. |
| Platform | Linux x86_64, built and tested in the devenv. No other platform is built or tested. |
| Rust toolchain | The devenv provides rustc 1.99.0. `Cargo.toml` declares `rust-version = 1.85` and edition 2024. Only the devenv toolchain is tested. |
| `scip` CLI | v0.10.0. The tests use it. |
| Stack Graphs fork | `Bullish-Intelligence/stack-graphs` at rev `32b5325ddc06f927dafd591ed1f1210e1f3bd8ed`. Four crates use one pin. |

Exit codes of `scippy index`: 0 complete, 3 finished but incomplete, 1 failure, 2 usage
(see "The result of `scippy index`").

### Known limits

The architecture notes list the open items (`SCIPPY_ARCHITECTURE.md`, section 11, in the notes
project, through the `.loci` link):

- A whole-string annotation such as `"Optional[Foo]"` is not followed. `Optional["Foo"]` is
  (fork item 31, section 11.1).
- The Python grammar is `tree-sitter-python` 0.23.5. The pack repairs PEP 758 `except A, B, C:`,
  PEP 750 `t"..."`, and PEP 696 type parameter defaults (`repaired-syntax`). A default is blanked, so
  the names inside it get no occurrence. A later grammar needs tree-sitter 0.25 in the fork.
- `a.b.c` chains need a type for the receiver. Pack rules follow written annotations only (ADR-009).
- The owner class differs from `scip-python` at 3,041 positions on mypy. The rebind convention
  differs at 5,028 positions.
- Base relationships to external (builtin) classes are not emitted.
- Identity evaluation limits: the truth is a proxy, only Python was measured at scale, few moves
  exist, and annotation survival is not measured (`identity-eval-2026-11-16/REPORT.md`, section 10).
- The metalayer makes no promise for entity IDs or the schema. A version change rebuilds the database.
- Slow tests run only on request, in `opt:slow`.

## Metalayer (experimental)

The metalayer is an experimental part of v0.1. Its goal is one index over notes and code for agent
consumers. It makes no stability promise for entity IDs or for the schema in v0.1.

Scippy can persist the facts of a run and reuse file graphs in two SQLite databases. With
`--metalayer`, it also keeps entity IDs in an identity store beside the metalayer database (see
`--identity` and `--no-identity`).
Create the database directory, then run:

```sh
mkdir -p ~/.local/state/scippy
scippy index PROJECT_ROOT --output index.scip \
  --metalayer ~/.local/state/scippy/metalayer.sqlite
```

The graph cache is on by default. `scippy index` uses the file
`$XDG_CACHE_HOME/scippy/graph-cache.sqlite`, or `$HOME/.cache/scippy/graph-cache.sqlite` when
`XDG_CACHE_HOME` is not set to an absolute path. It creates the folder on first use. One file
serves every project: each row has an owner made from the project root, the pack, the path, and
the dialect. Choose the cache in this order:

1. `--no-graph-cache` turns the cache off. `--graph-cache PATH` uses another file.
2. The variable `SCIPPY_GRAPH_CACHE` holds a path, or `off`.
3. The default file.

If the default file cannot be created or opened, or its lock times out, the run prints one
warning on stderr and goes on without the cache. A bad `--graph-cache PATH` or
`SCIPPY_GRAPH_CACHE` path does the same, but the program does not create its folder. The SCIP
output is the same with and without the cache. The cache is disposable: delete the file at any
time.

The graph cache stores each code file's stack graph and partial paths. A row holds the graph in
the `bincode` format, compressed with `zstd`. The cache key covers the
source, path, resolver settings, dialect, and executable. A new executable gets a new cache:
a run deletes the rows of other executables when it opens the file, so two executables must not
share one cache file. A debug build and a release build are two executables, so they replace
each other's rows in the default file. The cache is disposable.
The file uses the SQLite WAL mode and the `NORMAL` sync mode. A power loss can lose the newest
rows, and the cache then rebuilds them.
Each row records the run that last used it. After a run, if the file exceeds the cap, the cache
deletes the least recently used rows and returns the pages to the file system.
`--graph-cache-max-bytes` sets the cap. The default is 1G. Zero means no cap.
A cap smaller than the rows that one run needs makes each run rebuild some graphs.
Each row carries a checksum of its key and value. A row that fails the check is rebuilt.
A cache error turns the cache off for the rest of the run. The run prints one warning and goes on
without the cache. A damaged cache file moves to `<path>.bad`.

Every run still parses the files and resolves the references of every root. It also runs native
packs over the project. A file graph cache hit saves graph construction, not resolution.
The program prints the cache hit and build counts.

The metalayer is a set of typed SQLite tables of the facts of the latest run. The crate
`scippy-metalayer` owns it. The design is in `METALAYER_SCHEMA.md` and ADR-012. The SCIP file
stays the canonical output. The database is derived from the same run and a version change
rebuilds it.

- One database holds one root set. The key of the root set is the canonical path of the project
  root. The shards of a run are recorded in `root_member` and rewritten on each publish.
  A run on another project root fails and keeps the database. Add `--replace-root-set` to
  replace the root set.
- Each publish replaces all facts and commits one revision. The database keeps the latest
  revision only. The revision holds the input digest, the tool version, the SHA-256 of the SCIP
  bytes, and the flags `input_complete` and `analysis_exact`.
- The tables are `file`, `symbol`, `occurrence`, `reference`, `reference_candidate`, `edge`,
  `diagnostic`, and the relation `file_reference`. A reference row exists for every outcome:
  resolved, ambiguous, unresolved, and bounded. Spans are UTF-8 byte offsets. The store holds no
  source text. `View::source` reads a file again and checks its SHA-256.
- `file_reference` has one row for each file, target symbol, and kind. A support is one resolved
  reference location. It does not count when the target is a `local N` symbol, or when the
  location is the definition site of the same symbol. A same-file global target counts.
  A support that rests on a stopped search counts, and the column `limited` counts it too.
- The `edge` table has one row for each:
  - `import`: an import reference (an import occurrence at the span of a resolved reference);
  - `inheritance`: a class and one supertype, with the span of the name in the class header.
    A class with two supertypes has two edges;
  - `override`: a method and the method that it overrides;
  - `alias`: a name and the document that it refers to;
  - `link` and `embed`: a link or an embed of a native pack. The Loci pack passes the link kind
    (`wikilink`, `markdown_link`, `frontmatter_link`, `embed`, `markdown_embed`), the label,
    the state (`resolved`, `missing`, `ambiguous`, `skipped-target`), and the anchor state
    (`resolved`, `missing`, `unchecked`, `none`). A link `[[t#S]]` is one edge and has two
    supports: one for the note and one for the heading. An ambiguous link keeps its candidates
    in `reference_candidate`.
    A note over `note_max_bytes` is skipped, but its path still takes part in name resolution.
    A link that reaches only that note has the state `skipped-target`, the anchor state
    `unchecked` when it has an anchor, and the reference outcome `unresolved` with the reason
    `target-skipped`. `loci-core` has no such state: it drops the note, and the link is `missing`.
    The Loci oracle comparison maps `skipped-target` back to `missing`. An alias of a skipped
    note lives in its unread front matter, so a link by alias alone stays `missing`.
  A tag is a reference and has no edge.
- The column `reference.kind` says how a reference names its target in the source: `plain`,
  `member` (the `x` in `a.x`), or `keyword` (the `x` in `f(x=1)`). A reference of a native pack
  has no kind.
- The relation `file_reference` has the kinds `import`, `use`, `link`, and `embed`. A support
  is a link or embed support when an edge of that kind names its reference.
- `Metalayer::view` starts one read transaction. Its methods read one revision. Integer ids
  stay inside the database. `Metalayer::open_read_only` never writes, creates, or rebuilds.
- A database with an older format version is dropped and rebuilt. A database with a newer
  version is refused.
- `index_project_with_snapshot` returns the structured snapshot to a Rust caller.

Every run publishes, with a status for each file. The `file` table is the input manifest:

- `indexed`: the file became a document.
- `skipped`: Scippy does not accept the file. The reason is one of `symlink-skipped`,
  `file-too-large`, `input-limit-reached`, `unsupported-language`, `ambiguous-language`, and
  `invalid-utf8`. A file after the total input limit has a row too.
- `failed`: Scippy accepted the file and an operation failed. The reason is `unreadable-file`,
  `syntax-error`, or `bind-error`.
- `shadowed`: another file shadows it, so no import reaches it (`shadowed-file`). It is legal
  Python. The report counts it in `files_shadowed`, not in `files_skipped`.

The run reads each input file once, at its first use. The shards, the dependency loads, the
native packs, the snapshot text, and the SHA-256 of the manifest all use that one read. At the
end of the run, the pipeline hashes each indexed file on disk. A file whose hash differs from
the read is `stale`: the `file` row has `stale = 1`, and the diagnostic `stale-file` names it.
A stale file is indexed, not lost. Its facts come from the one read, so they agree with each
other and with the stored `sha256`. A stale file does not change `input_complete`.

A path with no valid UTF-8 name has no row. Its diagnostic (`non-utf8-path` or `invalid-path`)
makes the input incomplete instead.

The snapshot field `input_complete` is true only when the run lost no input. It is false when:

- a file has the status `skipped` or `failed`. A shadowed file does not count;
- a diagnostic has the severity `Error`;
- a diagnostic has the code `manifest-error`, `support-file-error`, `root-name-collision`,
  `file-too-large`, `non-utf8-path`, or `invalid-path`. They report a `pyproject.toml` or
  `tsconfig.json` that cannot be used, a dependency name that two roots share, a note over a
  size cap, and a path that has no valid name. These losses have no file row.

The snapshot field `analysis_exact` is true only when no reference search stopped at a limit.
It is independent of `input_complete`. A search that stops gives one of two results:

- the outcome `Bounded`: the reference has no target;
- the outcome `Resolved` with a `limited` mark: the nearest binding in the same document. Other
  ends can exist. This reference still supports the file-to-symbol relation.

The field `bounded_references` counts both kinds. `Metalayer::publish` accepts a snapshot with
`input_complete` or `analysis_exact` false. It computes both flags from the rows and returns an
error when the snapshot disagrees. The exit code of `index` is 3 when a run publishes lost input (see "The result of `scippy index`").
The revision records the flags, and the CLI prints the diagnostics.

`--strict` makes the run fail when `input_complete` is false, with or without `--metalayer`. An
error diagnostic already makes `input_complete` false. A failed run writes no SCIP file and opens
no database, and `ScippyError::Incomplete` is the error. The CLI prints the diagnostics first.
The revision records `strict`.

The identity store keeps entity IDs across edits, moves, and renames. v0.1 makes no stability
promise for these IDs or for the schema. The metalayer has no history, watcher, or query language.
The CLI publishes first and then writes the SCIP file. A failed SCIP write reports the
revision that the database already committed.
An edit during indexing does not mix two versions of a file. The indexer reads live files and
does not capture a filesystem snapshot, but it reads each file once. The manifests
(`pyproject.toml`, `package.json`, `tsconfig.json`), the Loci configuration, and the binary
check of the Loci listing are separate reads. They are not project sources in the snapshot.
Keep both databases outside the project and its roots.

### The result of `scippy index`

`scippy index --json` prints one envelope to stdout, in place of the text summary. It has
`format_version`, `tool`, `command` (`index`), `ok`, `input_complete`, `analysis_exact`, and
`value`. It has no `revision` block. A run with `--metalayer` reports its revision in
`value.metalayer`. The value holds `output`, `files`, `declarations`, `occurrences`,
`references`, `graph_cache`, `lost_files` (path, status, reason), `stale_files`, `diagnostics`,
and `metalayer`. A failed run prints an error envelope (`incomplete` for `--strict`). The
diagnostics also go to stderr. The text summary comes from the `Display` of `IndexReport`.

The `references` block holds `search_work`, the cost of the searches that resolve the references.
It has no time, so a test or a user can read the cost of a run on a loaded machine. The values
are the same for each run of the same input and limits, with any `--jobs`. A run with no
reference has zeros. The text summary does not show it.

| Field | Meaning |
|---|---|
| `searches` | The number of searches that returned counters. One search runs for each reference node, so it can exceed the number of reference locations. A failed search gives no counters and is not counted. |
| `work_total` | The work of all searches, added. One unit is one path that a search extends, or one candidate that it tries. |
| `work_max` | The work of the largest search. The `--max-work` limit stops a search soon after it reaches it, so this value can pass the limit by a few units. |
| `queue_length_max` | The longest queue of paths that waited for extension in one search. |
| `arena_cells_max` | The most cells that one search added to the path arenas. |

Exit codes of `index`, with or without `--json`:

| Code | Meaning |
|---|---|
| 0 | The run is complete: `input_complete` and `analysis_exact` are true. |
| 1 | The run failed. This includes `--strict` with lost input. |
| 2 | Usage error (clap). |
| 3 | The run finished, and it wrote its output, but it lost input or its analysis is inexact. |

Code 3 never occurs with `--strict` and lost input: that run fails with code 1 and writes
nothing. A script that accepts an incomplete run tests for the codes 0 and 3.

## Metalayer queries

`scippy metalayer <command> --db PATH [--json] [--limit N]` reads a metalayer database. It never
writes. The design and the JSON contract are in `METALAYER_QUERY.md`.

- `show` prints the revision, the root set, the flags, and the counts.
- `files` lists the input manifest. Filters: `--status`, `--language`, `--prefix`.
- `relations` lists the file-to-symbol relations. Filters: `--from`, `--to`, `--target-name`,
  `--kind`, `--external`, `--internal`.
- `edges` lists single edges: one site for each item.
- `explain --from PATH --to SYMBOL` shows why a file depends on a symbol: the relations, the
  supports with line and column, and the ambiguous references that list the symbol.
- `check` compares the files on disk with the stored hashes. Options: `--project-root`, `--scip`,
  `--discover`. The old name `--root` stays as a hidden alias for `--project-root`.

### The JSON contract

With `--json`, a command prints one envelope. The CLI output is the contract. The tables of the
database are private: never query them. The Rust API of `scippy-metalayer` is a secondary contract.

- `format_version` is an integer. It is 1 now. `schema_version` is the storage version. It is
  informational: do not switch on it.
- The envelope also holds `tool`, `command`, `ok`, `revision`, `root_set`, `input_complete`,
  `analysis_exact`, `position_encoding`, and `value`. A list value is `{items, total, truncated}`.
- An error envelope holds `ok: false` and `error: {kind, message}`. It has no `value`. The kinds
  are stable kebab-case strings.
- Key order is not part of the contract.

Compatibility policy (`METALAYER_QUERY.md`, section 5):

- An additive change keeps `format_version`: a new field, enum value, error kind, command,
  option, or filter.
- A breaking change raises `format_version`: the removal, rename, or retyping of a field; a
  change of the meaning or unit of a field, or of a sort order; an optional field that becomes
  required; a change of the meaning of an exit code.
- After a bump, the tool accepts `--format-version N` for the previous version for two minor
  releases. The changelog names each step.
- A reader must ignore unknown fields and unknown enum values. It treats an unknown value as an
  opaque string.

Exit codes, for every command:

| Code | Meaning |
|---|---|
| 0 | Success. For `check`: the index is fresh. |
| 1 | Error: bad database, schema mismatch, bad argument, I/O failure. |
| 2 | Usage error (clap). Text on stderr, no envelope. |
| 3 | `check` only: the index is stale. The envelope has `ok: true`. |
| 4 | The database holds no revision. Run `scippy index --metalayer` first. |

On exit 1, the error goes to stderr as `scippy: {error}`, or to stdout as an error envelope with
`--json`.

The JSON Schema (2020-12) is `crates/scippy-cli/schema/metalayer-v1.schema.json`. It refers to
one file for each command in `crates/scippy-cli/schema/metalayer-v1/`. It allows additional
properties. Every golden output must validate against it.

The golden files are in `crates/scippy-cli/tests/golden/query/`. To update them, run
`UPDATE_GOLDEN=1 cargo test -p scippy-cli --test query_golden`. A change in a golden file is a
contract change: check the policy above first.

A coding agent can use the three recipes of the skill `.agents/skills/scippy-metalayer/SKILL.md`:
who imports a symbol, why a file depends on a symbol, and whether the index is fresh. The test
`crates/scippy-cli/tests/query_golden/skill.rs` runs the recipes.

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
  byte-span positions, the SCIP emitter, and the structured analysis snapshot.
- `crates/scippy-metalayer` holds the metalayer store and its read API.
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
The pin is `32b5325ddc06f927dafd591ed1f1210e1f3bd8ed`. Commit `9a584b8` is an ancestor of the pin. It speeds up the stitcher and adds the option to turn off the debug
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

### Return annotations

A call of a function with a return annotation follows the annotation only. The `return`
statements of the body add nothing. The body stays visible when the annotation names no class:
`Self`, `Any`, `Type[...]`, a `TypeVar`, `ParamSpec`, or type alias that the same file binds, a type
parameter of a `def` or `class`, and a string annotation that holds more than a plain name,
such as `"Foo | None"`. A string with one plain name follows the annotation only.

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
- The finishing stage marks an Ambiguous reference as a union when every candidate is a method,
  an attribute, or a property of a class of the project, there are at most 4 candidates, and no
  owner class is one class with another owner, or a subclass or superclass of it (direct or
  through other classes). A union gets one occurrence for each candidate, in the order of the
  symbols. The store keeps the reference as ambiguous with its candidates. Override chains,
  module-level names, mixed kinds, members of external classes, and more than 4 candidates stay
  ambiguous with no occurrence. A base class that the project does not resolve gives no relation.
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

The fork also contracts the graph of each file at the end of a build. It prunes the scope nodes
that lead nowhere and merges the chains of plain scope nodes. On mypy this halves the arena cells
of the file phase and cuts the peak memory by 1.2 GB. Unlike the changes above, it can change the
output: on mypy it changes 47 locations, and 33 of them were bounded references that now resolve.

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
- `--resolve-threads N` sets the number of threads that resolve the references of one shard. The
  default is the value of `--jobs` divided by the number of shards that run at once, at least 1.
  So the shard threads times the resolve threads do not exceed `--jobs`. A project with one shard
  and `--jobs 8` resolves on 8 threads. The engine freezes the partial paths of the shard. Each
  thread works in its own overlay of them, takes the references in chunks of 16, and writes each
  result to a slot of its reference. The merge reads the slots in order, so the `.scip` file and
  the `--trace` file do not depend on `--resolve-threads`. The overlays add little memory: the
  peak resident memory of mypy stays at 4.2 GB.
- `--build-threads N` sets the number of threads that build the graphs of the files of one shard.
  The default follows the rule of `--resolve-threads`: `--jobs` divided by the number of shards
  that run at once, at least 1. A file builds without any other file. Workers parse the files and
  build the graphs of the files that the graph cache lacks, each in a stack graph of its own. The
  main thread looks each file up in the cache first, adds the graphs to the engine in file order,
  and stores the new graphs in the cache in file order. The workers share the compiled rules.
  A file that fails to build gives the same `bind-error` diagnostic at every thread count. The
  `.scip` file and the `--trace` file do not depend on `--build-threads`, with the graph cache off
  or on. More threads raise the peak memory: the finished graphs wait for the merge, and each
  worker holds a graph. The lookup runs at most 4 files for each thread, and at least 16 files,
  ahead of the merge, so few graphs wait. The peak of mypy rises from 3.9 GiB with 1 thread to
  5.5 GiB with 8 threads. The memory estimate of a shard rises with the threads, up to 1.5 times
  for its growing part. Use
  `--build-threads 1` to keep the memory of a serial build.

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
Then run the gate:

```sh
devenv shell -- testee verify
```

`testee verify` is the only gate. Run it before you land a change.
It runs `cargo fmt --check`, `cargo clippy -D warnings`, `cargo check`, and `cargo test --workspace`.
The gate runs the fast tests. Slow tests carry `#[ignore = "slow: runs in opt:slow"]`.
`opt:slow` also holds the performance and load-sensitive tests, which carry `#[ignore = "perf: runs in opt:slow"]`.
The default gate holds no test that asserts on time, speed, or resource use. The machine load makes such a test fail at random.
`opt:slow` runs only on explicit request. It is not part of any lane's verify step.
It is not part of the default gate.
The slow tests run `scip lint`, `scip snapshot`, and `scip test` on the Python, TypeScript, and Loci fixtures.
The Loci expectations come from `loci-core`. To make them again, run `scripts/loci-oracle.sh`.
During an edit, run the quick local check. It is not the gate:

```sh
devenv shell -- scripts/verify.sh
```

The script runs `testee verify --profile quick`: lint, format, and the library tests.
Set `SCIP_BIN_DIR` when `scip` is not in the default Testee tool path.
Set `UPDATE_GOLDEN=1` to rewrite the golden snapshots in `languages/python/tests/golden` and
`languages/typescript/tests/golden`, and `languages/loci/tests/golden`.
