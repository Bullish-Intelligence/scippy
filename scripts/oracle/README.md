# Accuracy oracle

The oracle compares the `.scip` index of Scippy with the index of scip-python 0.6.8 on standard
corpora. Scip-python is a reference, not the truth. The scripts never run an indexer on their own:
scoring reads stored indexes. Results are in `RESULTS.md`.

## Layout

All data lives in `ORACLE_DIR` (default `~/.local/state/scippy/oracle`). No corpus and no index is in
the repository.

| Path | Content |
|---|---|
| `corpora/NAME/root` | Source of corpus NAME: `.py` files only |
| `ref/NAME.scip` | scip-python 0.6.8 index of the corpus (empty environment, except pydantic-ai) |
| `baseline/NAME/` | Baseline run: `scippy.scip`, `scippy.trace`, `scip-python.scip`, `cmp/` |
| `scip-python/`, `env-empty.json` | The reference indexer and its empty environment |

Standard corpora (pins are in the header of `fetch_corpora.sh`):

| Name | Source | Use |
|---|---|---|
| mypy | 1.19.1 sdist | typed, large (187 files); the main gate |
| rich | commit 3a109cc | typed, small (100 files); the main gate |
| stdlib | CPython 3.14.7, 160 files of `Lib/` | mostly unannotated |
| scrapy | 2.6.3 sdist | mostly unannotated; 25 s of CPU, suits each change |
| celery | 5.3.6 sdist | dynamic patterns; has bounded references |
| pydantic-ai | commit ca52326, `pydantic_ai_slim/` | the long corpus |

Fetch the sources: `scripts/oracle/fetch_corpora.sh [NAME ...]`. The script checks the SHA-256 of each
archive and never runs corpus code. It skips a root that exists.

Build a reference index (only when a corpus is new or the scip-python version changes):
`scripts/oracle/run_tools.sh CORPUS CORPUS scip-python OUTDIR`, then copy
`OUTDIR/CORPUS/scip-python.scip` to `ORACLE_DIR/ref/CORPUS.scip`.
Setup of scip-python: see `run_all.sh`.

## Score a run

A run folder holds `scippy.scip` and `scippy.trace` (made with `scippy index --trace`).

    python3 -I scripts/oracle/score.py CORPUS RUNDIR [CMPNAME] [--label RUN]

It writes `RUNDIR/cmp/` (`summary.json`, `disagreements.jsonl`, `buckets.json`, `buckets.txt`,
`metrics.txt`, `metrics.json`). `--reuse-compare` keeps an existing `summary.json`.
`recompute.sh CORPUS OUTDIR` does the same for `OUTDIR/CORPUS`.

## Named numbers

`metrics.py` prints these numbers. There is no blended score. Each position is one that both tools
resolve in the project (`both`), or one that scip-python resolves (`sp`).

| Number | Meaning |
|---|---|
| strict recall, strict precision | the start of the target definition is equal; matched / `sp`, matched / `both` |
| overlap recall, overlap precision | the old raw numbers: the definition ranges overlap |
| owner-class coarse | scip-python gives `Class#`, Scippy gives a member of that class |
| import-prefix convention | `a` of `import a.b.c` or `a.b.X`: package (Scippy) or submodule (scip-python) |
| same-name other class | both give a member with the same name, in different classes |
| function type parameter | symbol convention for a type parameter of a function |
| scip-python first binding (`sp_first_binding`) | scip-python takes the first of two declarations of a name in one scope; Scippy takes a later live one (`enum.py`: `Enum = Flag = ... = None`, then `class Flag`) |
| real wrong | a different target that none of the classes above explains |
| scip-python builtin type attribute (`sp_builtin_type_attr`) | scip-python gives `builtins/type#` for an attribute named `type`; counted apart, it is not a position of `both` |

Old tables used "wrong" for the sum of real wrong and `sp_first_binding`, and a "plausible" blend.
`metrics.txt` prints both under "legacy numbers". Do not use them alone.

`analyze.py` assigns each disagreement to one class. Classes of `scippy_missing` have causes
(`cause.py`). `sp_class_path` reads both symbol forms of scip-python: `` `importlib.util`/X# ``
and `dataclasses/Field#`.

## Track classes per run

One CSV holds the counts of every run: `scripts/oracle/classes.csv`
(`run, date, corpus, kind, name, value`; kinds are `named`, `legacy`, `class`).

    python3 -I scripts/oracle/score.py CORPUS RUNDIR --label RUN        # score and append
    python3 -I scripts/oracle/track.py add RUN CORPUS=CMPDIR ...      # append scored dirs
    python3 -I scripts/oracle/track.py delta RUN --baseline BASE        # changed rows only
    python3 -I scripts/oracle/track.py runs

`add` refuses a (run, corpus) pair that the CSV already has. The baseline run is
`df9cecaa-2026-11-22`. Run `track.py delta` for each pin bump. For one location, use
`difftool.py CORPUS BASE_RUNDIR CAND_RUNDIR`: it lists the locations that are lost, changed, or
gained, and says if scip-python agrees with the base or with the candidate.

## Gate

The gate checks a candidate Scippy build against the baseline. It fails when:

1. real wrong is more than 0 on mypy or rich (`--zero-wrong`);
2. strict recall or strict precision falls by more than 0.05 pp (`--max-strict-drop`);
3. a location changes from resolved to unresolved, or to a different target, and `gate-allow.txt`
   does not list it;
4. an occurrence has more than one symbol.

The gate is not in `pm verify`: a mypy run takes minutes. Run it before you land a change to
the resolver, the rules, or the fork pin.

    SCIPPY_BIN=path/to/scippy scripts/oracle/run_gate.sh OUTDIR [CORPUS ...]   # default: rich scrapy mypy

`run_gate.sh` indexes each corpus with `--trace`, then runs
`python3 -I scripts/oracle/gate.py CORPUS=RUNDIR ...`. Use `gate.py` alone when the runs exist.
The exit code is 0 (pass), 1 (a check failed), or 2 (an input error).
The baseline of each corpus is `ORACLE_DIR/baseline/CORPUS` (`--baseline-dir` changes it).

Allowlist: after you decide that a change is right, add the line
`CORPUS<TAB>doc:LINE:COL<TAB>reason` to `gate-allow.txt`. `LINE` is 1-based and `COL` is a 0-based
code point offset, as the gate prints it. The gate reports allowlist lines that no longer change.
After you accept a new baseline, copy the new run to `ORACLE_DIR/baseline/CORPUS`, score it with
`--label`, and empty the allowlist.

## Tests

    python3 -I scripts/oracle/test_oracle.py

The tests use small synthetic indexes. They need no corpus and no indexer.
