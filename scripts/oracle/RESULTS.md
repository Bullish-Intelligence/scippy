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

## Run 2026-11-10: scip-python with a Python environment (`ORACLE_VENV=1`)

Scippy is main 9f588df (fork pin as in that commit). The Scippy indexes did not change.
Only the `scip-python` run changed. `mkenv.sh` built one environment per corpus with `uv`,
from wheels only: `pydantic` 2.14.3 and its dependencies, plus `typing_extensions` for
pydantic-ai. No corpus imports `pydantic_settings` or `sqlmodel`. No corpus code ran.
References that resolve into installed packages have no definition in the corpus index, so
they stay external. They count in neither recall nor precision.

Default mode (empty environment) did not move. The `scip-python` indexes of mypy and rich are
byte-identical to the earlier ones. A new Scippy run on main 9f588df gives the same metrics as
the table above: mypy raw recall 90.80%, without design gaps 94.30%, 21 wrong targets; rich
95.67%, 96.76%, 0 wrong targets.

Per corpus, empty environment, then environment. "Field keywords" is the class
`C_synth_init_field` (keyword names a dataclass, NamedTuple, or model field).

| Corpus | sp positions | Raw recall | Recall without design gaps | Wrong targets | Field keywords | Receiver type (C4/C5) |
|---|---|---|---|---|---|---|
| agentman | 6,507 -> 6,622 | 98.76% -> 97.04% | 98.79% -> 97.07% | 0 -> 0 | 0 -> 85 | 62 -> 92 |
| eventic | 8,062 -> 8,225 | 96.46% -> 94.55% | 96.51% -> 94.60% | 31 -> 31 | 1 -> 164 | 156 -> 156 |
| argentic | 15,242 -> 15,450 | 98.60% -> 97.27% | 98.63% -> 97.31% | 2 -> 2 | 0 -> 164 | 161 -> 205 |
| inferference | 32,982 -> 33,320 | 97.63% -> 96.64% | 97.79% -> 96.79% | 0 -> 0 | 18 -> 347 | 591 -> 600 |
| pydantic-ai | 160,140 -> 160,608 | 91.00% -> 90.74% | 92.11% -> 91.84% | 192 -> 192 | 1,323 -> 1,590 | 6,367 -> 6,571 |

Precision does not change. The environment adds recall positions that Scippy misses: keyword
names of model fields, and attribute reads on model instances (the receiver-type rise).
In pydantic-ai, the positions that both tools resolve in the project fall by 3 (147,277 to
147,274). The cause is not examined. The recall falls because the oracle now sees misses that it hid before.
Time of `scip-python` on pydantic-ai: 2:23 and 2.1 GiB with the environment.

## Run 2026-11-10: type parameters, type aliases, walrus, relative imports with many dots

Baseline is Scippy 9f588df with fork pin 55f33f4. The change is fork pin be7ece9 and the
declarations of the Python pack: type parameters (`typeparameter`), `type` aliases (`type`), and
the walrus name (`variable`). One index ran for each corpus and version, with `--trace`.
The `scip-python` indexes are the same for both versions.

Script change: bucket `function_type_parameter` is a plausible class. `scip-python` gives a type
parameter of a function the symbol of the function (`f().`). Scippy gives the type parameter, a
document-local symbol at its declaration in the `def` header. Scippy is correct by the language
rules. Without the class, 7 positions in eventic and 7 in argentic count as wrong.

| Corpus | Precision, plausible (before / after) | Wrong targets | Raw recall | Missing, Scippy |
|---|---|---|---|---|
| mypy | 99.99% / 99.99% | 21 / 13 | 90.80% / 90.85% | 8,966 / 8,891 |
| rich | 100.00% / 100.00% | 0 / 0 | 95.67% / 95.72% | 698 / 688 |
| pydantic-ai | 99.87% / 99.95% | 192 / 70 | 91.00% / 93.10% | 12,863 / 9,628 |
| eventic | 99.60% / 99.96% | 31 / 3 | 96.46% / 97.85% | 254 / 163 |
| argentic | 99.99% / 100.00% | 2 / 0 | 98.60% / 98.59% | 211 / 207 |

No position is wrong after the change that was right before. The remaining wrong targets are not
in the three syntax classes. Raw precision of argentic moves from 99.98% to 99.95%, because the 7
function type parameter positions are wrong in the raw count. The new class holds them.

Classes that the fixes touch (before / after):

- pydantic-ai, annotation names without a declaration (C2, no-declaration): 397 / 0.
- pydantic-ai, receiver is a parameter with a class annotation (C4b): 189 / 13.
- pydantic-ai, receiver bound by a walrus (C4 walrus): 210 / 139. Other walrus names moved out of
  `C_name_other` (110 / 0) and `C_name_store_target`.
- pydantic-ai, wrong targets W2 (name resolved to another local): 152 / 35.
- pydantic-ai, keyword argument names (C1): 29 / 17.
- eventic, annotation and name misses with no declaration: 56 / 0. Wrong targets 31 / 3.
- pydantic-ai: all 263 references on the 114 import lines with three or more dots resolve.

Run time and memory, mypy (GNU time): before 8:47.90 and 5.45 GiB; after 8:51.39 and 5.45 GiB.
pydantic-ai after: 6:29.50 and 5.92 GiB. The indexes ran one at a time.
The Loci oracle is unchanged: the Loci expectations and golden snapshots pass in `verify.sh`.

## Run 2026-11-10: model classes, TypedDict, and generic bases

Fork commit 013c27c. It adds the synthesized `__init__` for classes that derive from
`pydantic.BaseModel`, `pydantic_settings.BaseSettings`, or `sqlmodel.SQLModel` (also through bases
of the project, and from a base that `@dataclass_transform` marks), the keyword fields of
`TypedDict` classes, the decorators of attrs and `pydantic.dataclasses`, and the keyword paths of
`C[T](name=v)` and `class B(A[T])`.

Setup: `scip-python` runs with the environment (`ORACLE_VENV=1`, indexes in `evals/out-env`) for
the five user corpora. mypy and rich use the empty environment. The "before" Scippy is 9f588df.
The "after" Scippy is the fork at the parent of 013c27c plus these rules. It is the build that
isolates this change from the commits of the fork that landed at the same time.

| Corpus | Field keywords (C_synth_init_field) | Raw recall | Wrong targets | Precision, plausible |
|---|---|---|---|---|
| agentman | 85 -> 0 | 97.04% -> 98.32% | 0 -> 0 | 100.00% -> 100.00% |
| eventic | 164 -> 0 | 94.55% -> 96.55% | 31 -> 31 | 99.60% -> 99.61% |
| argentic | 164 -> 0 | 97.27% -> 98.33% | 2 -> 2 | 99.99% -> 99.99% |
| inferference | 347 -> 18 | 96.64% -> 97.63% | 0 -> 0 | 100.00% -> 100.00% |
| pydantic-ai | 1,590 -> 287 | 90.74% -> 91.58% | 192 -> 192 | 99.87% -> 99.87% |
| mypy | 0 -> 0 | 90.80% -> 90.80% | 21 -> 21 | 99.99% -> 99.99% |
| rich | 0 -> 0 | 95.67% -> 95.67% | 0 -> 0 | 100.00% -> 100.00% |

Syntax-only count (keyword arguments that name a field of an in-project model or TypedDict class,
`budget.py`): inferference 1,052, eventic 288, pydantic-ai 242, argentic 240, agentman 175.
Scippy resolved none of them before. Now it resolves all of them to the field, except one in
pydantic-ai. `kwresolve.py` also counts the TypedDict calls (merged by class name): 615 of the
about 640 TypedDict keywords in pydantic-ai resolve (the misses were about 561); the other 24
keyword arguments of pydantic-ai (model and TypedDict) have no occurrence. The 6 eventic calls
that `kwresolve.py` lists as another target name fields of classes that a function defines, so
their targets are `local` symbols and correct. No keyword has a target in the wrong class.

Field aliases (`Field(alias=...)`) occur in 4 fields in the corpora (pydantic-ai 1, argentic 3).
No call uses an alias as a keyword. The rules ignore aliases.

What remains in pydantic-ai (class C_synth_init_field, 287): calls inside `case` blocks of a
`match` statement do not resolve at all (the fork has no rule for the block; a probe confirmed
it). The rest is not examined. At the fork head with the other commit of the day (see below) the
class falls to 65. Keywords on methods of instances need the receiver type and are out of scope.

Cost. Time and memory are the same: pydantic-ai 6:07.79 (base 6:07.51), 6.19 GiB; mypy 8:36.96
and 5.43 GiB (base 8:38.44); rich 0:32.43. A first version also followed the generic base for
the lookup of members. It raised the raw recall of pydantic-ai by 0.6 point, but it hit the work
limit of the search for 275 more mypy references (182 more missing positions, recall -0.12 point) and it raised the CPU time
of pydantic-ai by 38%. The shipped version follows the generic base for keyword paths only.

Merged with main 1a76712 (it declares walrus names, type parameters, and type aliases; pin 013c27c).
A first measure on a base without 1a76712 gave extra wrong targets (rich 3, pydantic-ai 253). They
came from the fork commit be7ece9 without the Scippy declarations, not from this change. The table
below is the merged build. Wrong targets do not rise against main 1a76712 (mypy 13, rich 0,
pydantic-ai 70, eventic 3, argentic 0, agentman 0, inferference 0).

| Corpus | Field keywords (C_synth_init_field) | Raw recall | Wrong targets |
|---|---|---|---|
| agentman | 85 -> 0 | 97.04% -> 98.32% | 0 |
| eventic | 164 -> 0 | 94.55% -> 97.91% | 3 |
| argentic | 164 -> 0 | 97.27% -> 98.32% | 0 |
| inferference | 347 -> 18 | 96.64% -> 97.63% | 0 |
| pydantic-ai | 1,590 -> 65 | 90.74% -> 93.74% | 70 |
| mypy | 0 -> 0 | 90.80% -> 90.85% | 13 |
| rich | 0 -> 0 | 95.67% -> 95.72% | 0 |

Merged run time: pydantic-ai 6:28 and 5.9 GiB; mypy 8:53 and 5.4 GiB. The class falls from 287 to 65
in the merged build. The `case` block gap remains (a probe with the merged build confirmed it); the
rest of the fall is not examined.

## Run 2026-11-10: match statements

Fork commit 91450c8. The binding rules of the fork already bound the captures of a `case` and
resolved the subject and the body. They had no references for the names in a pattern. The commit
adds them. A class pattern (`case Point():`) references its class. A dotted name (`case Color.RED:`)
is a reference to the member. The keyword name of a class pattern (`case Point(x=0):`) is a member
reference to the class. It is no binding any more: before, it bound `x` as a variable.
Scippy declares the new bindings in `declarations.scm`: a single name in a `case`, a union, or a
keyword pattern (a capture), and the name of `as`. It classifies the pattern names after the first
as members.

Setup: `scip-python` indexes of the earlier runs are reused (empty environment for mypy and rich,
the pydantic environment for the five eval corpora). Before is main bdc99f3 with the pin 013c27c.
After is the fork at 91450c8 with these Scippy changes. Both Scippy builds ran with `--trace` and the
same `recompute.sh`.

| Corpus | Wrong targets | Raw recall | Case-clause references resolved |
|---|---|---|---|
| agentman | 0 -> 0 | 98.32% -> 98.32% | no `match` |
| eventic | 3 -> 3 | 97.91% -> 97.91% | no `match` |
| argentic | 0 -> 0 | 98.32% -> 98.32% | no `match` |
| inferference | 0 -> 0 | 97.63% -> 97.63% | no `match` |
| pydantic-ai | 70 -> 67 | 93.74% -> 94.01% | 2,495 -> 2,933 of 3,424 (72.87% -> 85.66%) |
| mypy | 13 -> 13 | 90.85% -> 90.85% | no `match` |
| rich | 0 -> 0 | 95.72% -> 95.72% | no `match` |

`caseblocks.py` counts the references of `scip-python` on the lines of a case clause (pattern,
guard, and body; 541 cases in 41 files of pydantic-ai) and the ones that Scippy resolves at the
same position. Only pydantic-ai uses `match`. The other corpora have none. The Loci oracle is
unchanged: `verify.sh` passes.

The earlier claim that no reference inside a `case` block resolved was too strong: 72.87% did.
What the old build missed was the class and the value names in the patterns, the keyword names of
class patterns, and the uses of the names that `as` and the keyword patterns bind.
The 65 keyword misses of the class `C_synth_init_field` in pydantic-ai are no `case` block
problem. The lines are plain calls such as `RequestUsage(unmeasured_requests=1)`, where the field
is in a base class that the rules do not read as a model class. They did not change.
What stays missing in the case clauses (491 references) has other causes: receiver types
(`str.lower`), rebinds that `scip-python` counts as references, and names from outside the project.

Known gap: the clauses of a `match` statement see the names of the clauses before them, as a
statement sees the bindings before it. A name that a case binds can then reach the body of a later
case. The wrong targets did not rise.

## Run 2026-11-10: performance

Four changes make the index faster and smaller. The output does not change.

1. The stitcher skips a candidate whose first four symbols clash with the path at the join node.
   The work count stays the same, so the work limit triggers at the same point.
2. The cycle detector keeps a Bloom filter of 1024 bits. It skips the list scan when no earlier
   path element starts at the end node. A filter of 4096 bits raised the peak memory of mypy to
   6.25 GB.
3. Scippy turns off the debug attributes of the fork (`tsg_location`, `tsg_variable`,
   `tsg_match_node`). The fork keeps them on by default.
4. The pipeline compiles the TSG rules once and shares them with the engine of each shard. The
   builtins still build once for each shard.

Before is main 89f4683 with the fork at 91450c8. After is the fork at 9a584b8 with these Scippy
changes. Both builds ran with `--trace` on the same machine, one index at a time, under
`/usr/bin/time -v`. Wall time is noisy, so the table shows user CPU time and peak resident memory.

| Corpus | User CPU (s) | Peak memory (GB) |
|---|---|---|
| agentman | 8.0 -> 5.2 (-36%) | 0.38 -> 0.31 (-19%) |
| eventic | 11.6 -> 7.2 (-38%) | 0.46 -> 0.31 (-32%) |
| argentic | 29.3 -> 20.2 (-31%) | 0.76 -> 0.56 (-26%) |
| inferference | 40.9 -> 26.0 (-36%) | 1.19 -> 0.82 (-30%) |
| rich | 32.8 -> 21.6 (-34%) | 0.90 -> 0.62 (-31%) |
| pydantic-ai | 389.7 -> 238.9 (-39%) | 5.94 -> 5.20 (-12%) |
| mypy | 533.1 -> 387.4 (-27%) | 5.45 -> 5.22 (-4%) |
| loci-roots | 3.1 -> 1.7 (-44%) | 0.23 -> 0.19 (-19%) |
| loci-deps | 5.8 -> 2.8 (-51%) | 0.24 -> 0.19 (-20%) |

For all seven corpora and two Loci workspace fixtures with roots (`roots` and `deps`; the second
one has four roots, so it indexes four shards), the `.scip` file and the `--trace` output of the
new build equal those of the old build, byte for byte.

## Run 2026-11-10: parallel shards

The core runs the shards of a workspace on worker threads. The engine types are not `Send`
(`StackGraphLanguage` shares its rules with `Rc`, and a support-file analyzer is a plain
`Box<dyn FileAnalyzer>`). The fork needs no change: each worker builds, uses, and frees its own
engines, and compiles the rules once for all of its shards. A worker makes its own support-file
analyzers. The scheduler starts the largest shard first and merges the results in shard order.

Workspace: `~/Notes` and its 57 roots (49 shards, 42,456 files, 12,594 indexed documents). Both
runs used `--trace`, read-only, one at a time under `index.lock`, on 4 cores with 8 threads and 125 GB
of memory. Sequential is main 75a50b7. Parallel is the new build with the defaults: 8 workers and
a budget of 16 GiB.

| Run | Wall time | User CPU (s) | System CPU (s) | Peak memory (GB) | Occurrences |
|---|---|---|---|---|---|
| Sequential, 75a50b7 | 14:13 | 818.5 | 26.1 | 8.05 | 1,405,585 |
| Parallel, 8 workers | 4:21 | 1051.9 | 30.3 | 10.87 | 1,405,585 |

The wall time falls to 31 percent (3.3 times faster). The user CPU time rises by 29 percent: the
workers share caches and memory bandwidth, and the machine has 4 cores with 2 threads each. The peak memory
rises by 2.8 GB, because the largest shards run together. The git status and HEAD of the vault
and the 57 roots were the same before and after.

The `.scip` file, the `--trace` file, the report, and the diagnostics of the parallel run equal
those of the sequential run at 75a50b7, byte for byte. The new build with `--jobs 1` also gives
the same `.scip`, `--trace`, and report. On the fixtures `roots` and `deps`, `--jobs 1`, `--jobs 4`,
and a budget of one byte give the same files as 75a50b7. A test of the repository checks it.

Calibration of the memory estimate. `SCIPPY_SHARD_LOG=1` with `--jobs 1` gave the peak resident
memory after each shard. The numbers include the memory that earlier shards left in the heap, about
0.2 to 1 GB at the end of the run, so they are upper bounds for small shards.

| Shard | Python MB | Peak (GB) | GB per MB |
|---|---|---|---|
| llama-infernal | 2.7 | 7.7 | 2.8 |
| semantic-views | 2.3 | 4.1 | 1.8 |
| loci-core | 1.8 | 1.5 | 0.8 |
| argentic | 1.3 | 0.7 | 0.6 |
| pynix | 15.7 | 4.0 | 0.25 |
| mypy (corpus) | 4.7 | 5.2 | 1.1 |
| pydantic-ai (corpus) | 9.6 | 5.2 | 0.5 |

The ratio falls as the shard grows. The peak of the large shards stays between 4 and 8 GB. The
estimate is therefore 300 MiB plus 1.3 GiB for each MiB of loaded source, and the second part stops
at 7.7 GiB. A linear estimate of 1.3 GiB for each MiB gives 19 GB for pynix, which would run alone
under the 16 GiB budget, so it would delay the other shards. The estimate is too low for
llama-infernal (3.9 GB against 7.7 GB). The budget is half of the physical memory, so it absorbs an
error of a factor of two.

The corpora without roots run as before. For mypy and pydantic-ai, the new build gives the same
`.scip`, `--trace`, and report as main 75a50b7, byte for byte. A project with one shard runs on
the calling thread.

## Memory estimate and shard log (branch memfix)

The old estimate (300 MiB plus 1,300 bytes for each source byte) was low for llama-infernal (3.9 GB
against a peak of 8.08 GB) and high for pynix (8.4 GB against 4.2 GB). The new estimate is 300 MiB
plus 3,000 bytes for each source byte. The second part stops at 10 GiB, above the largest peak.
Llama-infernal now has 8.54 GB, pynix 11.05 GB, and semantic-views 10.6 GB.

`SCIPPY_SHARD_LOG=1` no longer writes `/proc/self/clear_refs`. A sampling thread reads `VmRSS`
every 10 ms and the log prints the start and the peak of each shard. `time -v` and an independent
`VmHWM` sample now agree. The figures are for the whole process, so they are exact for one shard
only with `--jobs 1`.

| Run | Wall time | User CPU (s) | System CPU (s) | `time -v` peak (GB) | `VmHWM` sample (GB) |
|---|---|---|---|---|---|
| `--jobs 1`, `SCIPPY_SHARD_LOG=1` | 14:46 | 840.2 | 26.8 | 8.04 | 8.04 |
| Default parallel | 7:19 | 916.3 | 28.6 | 9.48 | 9.48 |

The `.scip`, `--trace`, and stdout of the two runs are byte-identical. The git status and HEAD of
the vault and the roots were the same before and after each run.

No shard grew past its estimate. Fifteen small shards have a peak above their estimate, because
their start figure already holds the heap that earlier shards left (0.5 to 1.2 GB at the end of
the run). The growth of each (peak minus start) stays under the estimate. The parallel run is
slower than the run at b8b522b (4:21) because pynix and semantic-views now each take most of the
12 GiB budget and run nearly alone.
