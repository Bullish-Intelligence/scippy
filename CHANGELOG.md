# Changelog

All notable changes to Scippy are in this file.
The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Added

- The Python pack repairs three constructs of Python 3.14 that `tree-sitter-python` 0.23.5 rejects, and gives the warning `repaired-syntax` instead of `syntax-error`: PEP 758 `except A, B, C:` and `except* A, B:` (the commas become `|`), PEP 750 `t"..."` (the prefix becomes `f`), and PEP 696 type parameter defaults (the default is blanked, so the names inside it get no occurrence). On `semantic-views`, 17 files that Scippy skipped now index.
- A test in `opt:slow` guards the search of an annotated factory chain. The fixture `annotated_chain` has a chain of six factories with `-> Address` and 20 member chains. It asserts 0 bounded references and the target of each use. With the fork pin `cda555a` it fails with 20 bounded references.

### Changed

- The fork pin moves to `930f3e2`. The fork narrows more `isinstance` guards, follows the return annotation of a string that holds one plain name, and keeps one more similar path at an attribute store. This restores most members that the pin `3240d66` lost. On mypy, the raw recall rises from 94.19% to 94.24%, and 21 of the 27 locations that `3240d66` lost resolve again. On rich, the raw recall rises from 96.78% to 96.80%, and the bounded references fall from 4 to 0. There are no wrong targets. On mypy, 3 locations go from resolved to ambiguous.
- The message of the `repaired-syntax` diagnostic says that a name in blanked text gets no occurrence.

- The fork pin moves to `3240d66`. The fork contracts the graph of each file, and a call of an annotated function follows the return annotation only. On mypy, the bounded references fall from 330 to 0, the raw recall rises from 94.11% to 94.19%, the wall time falls from 1,515 s to 533 s, and the peak memory falls from 5.45 GB to 4.22 GB. There are no wrong targets.
- `metalayer show` no longer prints `parent`; facts schema 10.
- `scippy metalayer check` takes `--project-root`. The old `--root` stays as a hidden alias.
- Metalayer facts schema version 9: the unused `root_id` column is gone. Older facts databases rebuild on open.

### Fixed

- A lambda parameter is a local symbol of its own, not the parameter symbol of the enclosing function (item 32).

## [0.1.0] - 2026-11-16

### Added

- `scippy index` writes a SCIP index of a project, with deterministic output and atomic writes.
- Python pack: definitions, imports, references, builtins and stubs as external symbols, and override relationships.
- Python resolution with the Stack Graphs fork: cross-file names, packages, aliases, `isinstance` narrowing, class keyword arguments, container element types, and `Optional[...]` annotations.
- Loci Markdown pack: notes, headings, blocks, links, and embeds, with the same states as `loci-core`.
- Loci roots, `roots_file`, and the ignore list: one index over notes and code repositories.
- TypeScript pack as a calibration guard (ADR-011). It is not supported.
- Parallel shards with `--jobs` and `--memory-budget`.
- Graph cache of file graphs, on by default.
- Experimental metalayer (SQLite): facts of each run, with a status for each file.
- `scippy metalayer` commands: `show`, `files`, `relations`, `edges`, `explain`, and `check`, with a versioned JSON envelope.
- Experimental identity store: entity IDs rebound across runs (tiers T1 to T5 for code, M2 to M5 for notes).
- `scippy metalayer entity` and `annotate` commands: `add`, `list`, `rm`, `status`, `accept`, and `reject`.
- `scippy index --json` and `--strict`.
- `scippy index` exit codes: 0 complete, 3 finished but incomplete, 1 failure, 2 usage.
- Accuracy oracle against `scip-python` (`scripts/oracle`).
- Testee gate and the `opt:slow` task for slow tests.
- Gate 6 conformance tests: core language names, invalid rules, a mixed run, creation order, and Loci CRLF notes.
- Oracle rerun for v0.1: Python on mypy and rich, Loci on the real vault.

### Changed

- The graph cache is on by default in the CLI.
- The default memory budget is a flat 50 GiB.
- The builtins and stubs graphs are built at build time and loaded in each run.
- The pipeline and the metalayer publish code are split into named modules and phases.
- `scippy index` is one entry point with an `IndexMode` value.
- The tests run with the `gate` profile.
- Identity tier T3 needs an equal body digest for a top-level class. Two classes with the same name no longer bind as moved.
- `scippy --version` prints `scippy 0.1.0`.

### Known limits

- The metalayer and the identity store are experimental. Entity IDs and the schema can change.
- Lambda parameters share the parameter symbol of their enclosing function (fork item 32).
- A whole-string annotation such as `"Optional[Foo]"` is not followed.
- The identity evaluation measured Python at scale only. Other languages have few cases.
- The tests cover Linux x86_64 only.
- The Loci comparison with `loci-core` was not rerun after the note, heading, and block change.
