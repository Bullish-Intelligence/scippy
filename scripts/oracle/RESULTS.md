# Oracle results, corrected for the import-prefix artifact

Date: 2026-11-09.

## Method

- The oracle compares Scippy with scip-python 0.6.8 by definition location.
- Corpora: mypy 1.19.1 sdist (187 files) and rich 3a109cc (100 files).
- Scripts: `scripts/compare.py`, `analyze.py`, `metrics.py`. Backups: `*.orig`.
  `scripts/recompute.sh OUTDIR CORPUS` recomputes without a new index run.
- Inputs: the existing `.scip` files. No tool ran again.
- Scippy versions:
  - baseline e61310a9: `oracle/out`
  - f3c39830: `g3b/oracle/out-main`
  - 645af584: `g3b/oracle/out`

## The correction

For `import a.b.c`, Scippy binds the leading name `a` to package `a`.
scip-python points it at the submodule. Both readings are plausible.
A new class, "both plausible: import prefix", holds two kinds of position.
Both are module-name components in an import statement (`import a.b.c`, or
the module of `from a.b import x`).

Decision 2026-11-09: the same case at a usage site also counts as "both plausible".
Example: in `mypy.nodes.GDEF`, Scippy links `mypy` to the package and scip-python
links it to the submodule. Python binds the leading name to the package, so
Scippy's target is correct. The old scripts labelled this case W4 and counted it wrong.
Now it sits in the class "both plausible: import prefix" as the sub-kind
"import prefix at usage sites". Backups of the previous scripts and results: `*.w4bak`.

1. Different target: Scippy points at a package (`a/__init__.py`) that is a path
   prefix of the scip-python target module. This counts as plausible, so it counts as correct.
2. No scip-python occurrence: Scippy emits a module symbol. This no longer counts as coverage.

Every other rule is unchanged.

## Table: mypy

| Metric | e61310a9 | f3c39830 | 645af584 |
|---|---|---|---|
| Precision (raw) | 97.74% | 96.99% | 96.36% |
| Precision, plausible counted correct | 99.98% | 99.99% | 99.98% |
| Real wrong targets | 25 (0.019%) | 11 (0.008%) | 23 (0.017%) |
| Raw recall | 83.19% | 83.64% | 87.15% |
| Recall without design gaps | 95.17% | 95.28% | 95.37% |
| Coverage, raw | 0.74% | 2.45% | 2.43% |
| Coverage, without the artifact | 0.74% | 1.60% | 1.62% |
| Builtin agreement (same name / both) | 99.67% | 99.67% | 99.66% |
| Import-prefix class, import statements (different / no occurrence) | 0 / 0 | 1,030 / 1,158 | 1,030 / 1,158 |
| Import-prefix class, usage sites (was W4) | 0 | 0 | 782 |

## Table: rich

| Metric | e61310a9 | f3c39830 | 645af584 |
|---|---|---|---|
| Precision (raw) | 99.64% | 99.00% | 98.95% |
| Precision, plausible counted correct | 99.95% | 99.98% | 99.96% |
| Real wrong targets | 8 (0.050%) | 4 (0.025%) | 8 (0.043%) |
| Raw recall | 78.32% | 78.39% | 89.73% |
| Recall without design gaps | 93.50% | 93.56% | 95.50% |
| Coverage, raw | 0.12% | 2.86% | 2.69% |
| Coverage, without the artifact | 0.12% | 0.32% | 0.37% |
| Builtin agreement (same name / both) | 99.76% | 99.76% | 99.78% |
| Import-prefix class, import statements (different / no occurrence) | 0 / 0 | 108 / 423 | 109 / 442 |
| Import-prefix class, usage sites (was W4) | 0 | 0 | 0 |

Notes:

- Baseline has no module occurrences, so the new class is empty there.
- Wrong targets are counted against the positions both tools resolve.
- "Design gaps" are keyword names, attribute stores, module references, rebinds,
  coarse owner-class targets, and skipped files. The scripts decide this set.
- W4 was the label for the import-prefix case at a usage site. The 2026-11-09 decision
  moves it into the plausible class. At 645af584 on mypy, 782 positions moved from
  wrong to plausible. No other version or corpus has W4 positions, so only
  645af584 on mypy changed. Raw precision is unchanged (96.36%).
- Run time and memory, mypy (GNU time): scip-python 3:45.84 and 1.44 GiB.
  Baseline Scippy 9:17.62 (2.47x) and 6.00 GiB. 645af584: 8:34.64 (2.28x) and 6.07 GiB.
  f3c39830 has no clean timing run.
- Skipped files: mypy 0 of 187 at every version. rich: baseline and f3c39830 skip 1 of 100
  (the `x = yield` bind error). 645af584 has all 100 documents.

## Targets (soft targets, not kill limits)

Precision uses the plausible-counted-correct figure. The raw figure shows in parentheses.
Values for the corpora are mypy, rich.

| Target | Baseline | f3c39830 | 645af584 |
|---|---|---|---|
| Precision >= 97% | pass (mypy raw pass) | pass (raw 96.99%: 0.01 below) | pass: 99.98%, 99.96% (raw fails: 96.36%, 98.95%) |
| Wrong-target rate <= 0.5% | pass | pass | pass: 0.017%, 0.043% |
| Raw recall >= 75% | pass | pass | pass: 87.15%, 89.73% |
| Recall without design gaps >= 90% | pass | pass | pass: 95.37%, 95.50% |
| Coverage <= 2% | pass | mypy raw fail 2.45%, rich raw fail 2.86%; corrected pass | corrected pass (1.62%, 0.37%); raw fail (2.43%, 2.69%) |
| Skipped files <= 1% | rich 1.0%: pass at the limit | rich 1.0%: pass at the limit | pass (0%) |
| Time <= 3x scip-python on mypy | 2.47x pass | not measured | 2.28x pass |
| Memory <= 8 GB on mypy | pass | not measured | 6.07 GiB pass |
| Precision loss vs baseline <= 0.5 point | n/a | raw: mypy -0.75, rich -0.64 fail; plausible: +0.01, +0.03 pass | raw: -1.38, -0.69 fail; plausible: mypy 0.00, rich +0.01 pass |

With the decision applied, every soft target passes at 645af584 on the plausible
precision basis. Two raw figures still miss: raw precision and raw coverage. New
positions (module references, parameter symbols) land in the plausible classes and
in the import-prefix coverage class, so the raw figures fall. The plausible and
corrected figures are the meaningful ones.

## Run 2026-11-10: new scripts, synthesized `__init__`, star keywords, `global` and `nonlocal`

Baseline is Scippy main 577b134 with fork pin e5cba0a. The change is fork pin 55f33f4.
The scripts moved to `scripts/oracle` in the Scippy repository. On the baseline indexes, the
repository copy gives the same metrics and buckets as the state-folder copy.

Script changes:

- Raw recall counts a rebind as matched when the Scippy definition symbol has the definition
  location of the scip-python target (rebind credit). The rest stays in `C_rebind`.
- A keyword argument that Scippy resolves to `*args` or `**kwargs` is a wrong target (W5).
- `C_sp_coarse`, keyword names of method parameters, and attribute stores move to the
  receiver-type family (C4/C5). Keyword names of a class field go to `C_synth_init_field`.
  None of these is a design gap now. `C_module_ref` left the design list (it is 0).
- Design gaps are now `C_rebind` without credit, `C1_keyword` of plain functions, and skipped files.

| Metric | mypy old scripts, before | mypy old, after | mypy new, before | mypy new, after | rich old, before | rich old, after | rich new, before | rich new, after |
|---|---|---|---|---|---|---|---|---|
| Precision, plausible counted correct | 99.99% | 99.99% | 99.96% | 99.99% | 99.99% | 100.00% | 99.99% | 100.00% |
| Wrong targets | 21 | 21 | 64 | 21 | 1 | 0 | 1 | 0 |
| Keyword to star parameter (W5) | - | - | 43 | 0 | - | - | 0 | 0 |
| Raw recall | 87.55% | 87.67% | 90.68% | 90.80% | 91.22% | 91.91% | 94.97% | 95.67% |
| Recall without design gaps | 95.75% | 95.76% | 94.18% | 94.30% | 97.00% | 97.05% | 96.06% | 96.76% |
| Synthesized `__init__` class | - | - | 135 | 0 | - | - | 136 | 0 |
| Receiver type (C4/C5) | - | - | 6,995 | 6,995 | - | - | 475 | 475 |
| of which keyword on method parameter | - | - | 165 | 165 | - | - | 31 | 31 |
| `C_rebind` without credit / rebind credit | - | - | 211 / 4,817 | 211 / 4,817 | - | - | 30 / 767 | 30 / 767 |
| Coverage, raw | 2.44% | 2.44% | 2.36% | 2.36% | 3.08% | 3.09% | 2.96% | 2.98% |

Run time and memory of the new Scippy (GNU time): mypy 8:38.44 and 5.41 GiB (baseline 8:27.63
and 5.43 GiB); rich 0:34.49 and 0.88 GiB. The indexes ran one at a time.
The `global` and `nonlocal` rules touch 8 statements in the two corpora, so no metric shows them.
