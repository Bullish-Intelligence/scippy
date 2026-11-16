# Changelog

All notable changes to Scippy are in this file.
The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [0.1.0] - UNRELEASED

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

### Changed

- The graph cache is on by default in the CLI.
- The default memory budget is a flat 50 GiB.
- The builtins and stubs graphs are built at build time and loaded in each run.
- The pipeline and the metalayer publish code are split into named modules and phases.
- `scippy index` is one entry point with an `IndexMode` value.
- The tests run with the `gate` profile.

### Known limits

- The metalayer and the identity store are experimental. Entity IDs and the schema can change.
- Lambda parameters share the parameter symbol of their enclosing function (fork item 32).
- A whole-string annotation such as `"Optional[Foo]"` is not followed.
- The identity evaluation measured Python at scale only. Other languages have few cases.
- The tests cover Linux x86_64 only.
- The Loci comparison with `loci-core` was not rerun after the note, heading, and block change.
