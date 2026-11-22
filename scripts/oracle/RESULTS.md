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
the run). The growth of each (peak minus start) stays under the estimate. The parallel run of this
table used the old default budget of 16 GiB (the machine has 125 GB of memory).

The default budget is now a flat 50 GiB. The shard comparison uses growth (peak minus start), not
the raw peak. A shard over the budget still runs alone.

| Run | Wall time | User CPU (s) | System CPU (s) | Peak memory (GB) |
|---|---|---|---|---|
| Default parallel, 50 GiB budget | 3:30 | 1243.5 | 32.8 | 16.02 |

The `.scip` and `--trace` equal those of the `--jobs 1` run above, byte for byte, and the vault
and the roots had the same git status before and after. The wall time falls from 7:19 to 3:30,
and the peak memory rises from 9.5 to 16.0 GB, because the largest shards now run together.

## Wrong-target fixes (branch wrongfix)

Before is main 281a29f with the pin 9a584b8. After is the fork d39e253 and the change of
`reconcile.rs`. The five eval corpora ran with the pydantic environment for `scip-python`. Mypy and
rich ran with the empty one. The `scip-python` indexes are the same in both runs.

| Corpus | Wrong targets | Raw recall |
|---|---|---|
| mypy | 13 -> 0 | 90.85% -> 91.85% (139,925 -> 141,463 of 154,016) |
| rich | 0 -> 0 | 95.72% -> 95.85% |
| agentman | 0 -> 0 | 98.32% -> 98.43% |
| eventic | 3 -> 3 | 97.91% -> 97.91% |
| argentic | 0 -> 0 | 98.32% -> 98.53% |
| inferference | 0 -> 0 | 97.63% -> 97.79% |
| pydantic-ai | 67 -> 44 | 94.01% -> 94.27% (150,991 -> 151,411 of 160,608) |

The Loci oracle is unchanged: the Loci golden files and expectations pass in `scripts/verify.sh`.

What changed:

1. Store to an attribute (mypy 13, pydantic-ai 17). The path of a member reference can pop the
   name of the reference at an assignment target that no declaration claims, as in
   `response.model_name = self._model_name` with an unknown type of `response`. The path then
   follows the assigned value, and its end is the right-hand side. `RawEnd.echo` records these
   definition nodes. The reconciler drops such an end, also when no other end exists. All 13
   mypy wrongs and the 17 of pydantic-ai are gone.
2. Keyword capture in a class pattern. Main already binds the capture token (`a` in
   `case P(x=a)`), and the uses of the capture resolve to it. The e2e test
   `binding_forms_bind_the_right_token` now checks columns. The 29 wrongs of the audit are the
   key tokens: `scip-python` links a key with the same name as its capture
   (`capability_id=capability_id`) to the capture. Scippy links the key to the field, which is
   right. They stay in the count, and no change is made. Skipped on purpose.
3. Star target `*words, last = value` and the loop form `for a, *b in rows`: bound (fork rule and
   `declarations.scm`). The 4 wrongs of `models/test.py` are gone.
4. Tuple or list target of `with ... as (a, b)`, also nested or after a star: bound. The rules give
   no value to the names, because the element of the value is not known.
5. The `else` clause of a `try` statement sees the names of the body. Its edge has precedence 1,
   so a name bound in the body shadows the binding before the statement. The case `{...} as x` and
   the captures inside a dict pattern value already resolved on main (checked with a fixture).
6. Work limit. A bounded search keeps its result for a plain name when it has a nearest binding
   (no value flow) that is one declaration of the same document and no other nearest end
   exists. The first try kept only local symbols: mypy 139,935 -> 141,219 matches, 0 new wrongs.
   Allowing any declaration of the same document (parameters too) gave 141,463 in mypy, and
   pydantic-ai 151,318 -> 151,411 over the first try, with the same wrongs and the same plausible
   count. The shipped rule is the second.
   Member references, keyword names, and the queue and arena limits stay bounded.

The per-corpus gain without item 6 (mypy +10 matches, rich +11, agentman +7) comes from items 1
to 5. The wall times of this run are no measure: other runs shared the machine.

## isinstance narrowing (branch narrow)

Before is main 5342f7c with the fork d39e253. After is the fork with the narrowing rules. The five
eval corpora ran with the pydantic environment for `scip-python`. Mypy and rich ran with the empty
one. The `scip-python` indexes are the same in both runs. Ambiguous is the count of
`references ambiguous` in the Scippy report.

| Corpus | Wrong targets | Raw recall | Ambiguous |
|---|---|---|---|
| mypy | 0 -> 0 | 91.85% -> 93.22% (141,463 -> 143,573 of 154,016) | 254 -> 363 |
| rich | 0 -> 0 | 95.85% -> 95.88% | 22 -> 24 |
| agentman | 0 -> 0 | 98.43% -> 98.44% | 2 -> 2 |
| eventic | 3 -> 3 | 97.91% -> 98.61% | 16 -> 16 |
| argentic | 0 -> 0 | 98.53% -> 98.55% | 16 -> 16 |
| inferference | 0 -> 0 | 97.79% -> 97.79% | 12 -> 12 |
| pydantic-ai | 44 -> 44 | 94.27% -> 94.94% (151,411 -> 152,487 of 160,608) | 182 -> 153 |

The Loci oracle is unchanged: the Loci golden files and expectations pass in `scripts/verify.sh`.

Cost, one `scippy index` run without `--trace`, base then new back to back. The machine ran other
agents, so the user time of one build varies by 10% between runs.

| Corpus | Wall time | User CPU (s) | Peak memory (GB) |
|---|---|---|---|
| mypy, run 1 | 8:13 -> 6:59 | 437.9 -> 410.3 | 5.21 -> 5.24 |
| mypy, with trace (oracle run) | 7:24 -> 6:51 | 377.1 -> 402.0 | 5.22 -> 5.24 |
| pydantic-ai | 3:53 -> 4:02 | 224.3 -> 234.1 | 5.16 -> 5.20 |
| argentic (quiet machine) | 19.9 s -> 20.5 s | 19.5 -> 20.0 | 0.53 -> 0.55 |

The time stays inside the noise on mypy and grows 4% on pydantic-ai.

How it works. Each statement of a module or block has a narrowing scope that leads to the scope of
the enclosing statement. A guard adds an edge from the scope of each narrowed statement. The edge
pops the subject (`x`, or `a.b`) and pushes an instance of the class. The receiver of an attribute
looks in the narrowing scope with a precedence above the other scopes. So the narrowed class
shadows the declared type when it has the member, and the declared type resolves the rest. The
scope belongs to the statements, not to the flow of bindings, so nothing leaks out of the guarded
block. A tuple of classes gives one candidate for each class.

A first build skipped `mypy/checkexpr.py` (9,000 positions lost). A comment in the argument list of
`isinstance(...)` (`# type: ignore`) broke the pattern that defined the narrowing nodes, and a
stanza then used an undefined node. Every `isinstance` call now defines the nodes. A fork test
covers this case. Before the fix, the first build had 31 wrong targets and 86.17% recall on mypy (base 13 and 90.85%, main 281a29f).

The mypy ambiguous count rises by 109. A tuple of classes gives one candidate for each class, so
its members can be ambiguous. This run did not break the 109 down.

## Run 2026-11-11: element types of containers

Fork commit cda555a on 41d89fb; Scippy on main e772606. The rules read the name of a written container
annotation (`list`, `Sequence`, `Iterable`, `Iterator`, `set`, `frozenset`, `deque`,
`tuple[T, ...]`, `dict`, `Mapping`, `defaultdict`, with `typing` spellings, `Optional`, and `| None`).
A loop or comprehension asks for `<elem>`. A subscript asks for `<val>`. `d.items()`, `d.values()`,
`d.get()`, `enumerate`, `zip`, `sorted`, and `*args: T` also keep the element types. The earlier run
on 9a584b8 had three new wrong targets in mypy (`copy_modified(id=...)` after an `isinstance` guard).
They are gone now that the receiver narrowing is in main.

Setup: scip-python indexes of the earlier runs, the pydantic environment for the five eval corpora.
Before is main e772606; after is the same with the new fork rules. Both ran with `--trace`.

| Corpus | Wrong targets | Raw recall | for_target class | assign_subscript class |
|---|---|---|---|---|
| mypy | 0 -> 0 | 93.22% -> 93.82% | 727 -> 194 | 191 -> 72 |
| rich | 0 -> 0 | 95.88% -> 96.72% | 124 -> 45 | 27 -> 4 |
| pydantic-ai | 44 -> 44 | 94.94% -> 95.60% | 849 -> 461 | 120 -> 55 |
| eventic | 3 -> 3 | 98.61% -> 99.70% | 46 -> 0 | 10 -> 1 |
| argentic | 0 -> 0 | 98.55% -> 99.00% | 26 -> 22 | 11 -> 0 |
| inferference | 0 -> 0 | 97.79% -> 98.78% | 178 -> 24 | 35 -> 5 |
| agentman | 0 -> 0 | 98.44% -> 98.70% | 33 -> 18 | 2 -> 0 |

Cost (timed runs, no other load on the machine):

| Corpus | Wall time | User CPU (s) | Peak memory (GB) |
|---|---|---|---|
| mypy | 9:59 -> 9:06 | 530 -> 525 | 5.44 -> 5.45 |
| pydantic-ai | 4:07 -> 4:29 | 238 -> 260 (+9%) | 5.48 -> 5.52 |

The Loci oracle is unchanged: `verify.sh` passes. Left open: a container class of the project, a dict
subclass with type arguments in its base class, and the types of values that need `isinstance`
or value flow.

## Run 2026-11-16: v0.1 release record

Scippy `main` at `554f5321`. The code equals `68bdda69`: the commits after it change tests and the
identity rebind in `scippy-metalayer` only. The binary is a release build in a scratch target
directory. The fork pin is the one in that commit.

Method as in the earlier runs. Corpora: mypy 1.19.1 sdist (187 files) and rich (100 files). The
`scip-python` 0.6.8 indexes of 2026-11-10 (empty environment) are reused, because the corpora and the
tool did not change. Scripts: `run_tools.sh`, `run_trace.sh`, `recompute.sh` in this folder.
The mypy `--trace` run read the graph cache that the first run wrote (187 cached).

| Metric | mypy 2026-11-11 | mypy now | rich 2026-11-11 | rich now |
|---|---|---|---|---|
| Wrong targets | 0 | 0 | 0 | 0 |
| Precision, plausible counted correct | 100.00% | 100.00% | 100.00% | 100.00% |
| Raw recall | 93.82% | 94.11% (144,939 of 154,016) | 96.72% | 96.77% (19,752 of 20,411) |
| Recall without design gaps | - | 98.44% | - | 97.87% |
| Coverage, raw / without the artifact | - | 2.64% / 1.89% | - | 2.95% / 0.80% |
| Import-prefix class (statements / usage sites) | - | 1,030 / 782 | - | 109 / 0 |
| References bounded | - | 330 | - | 4 |
| Skipped files | 0 | 0 | 0 | 0 |

The plausible precision of the earlier release record (2026-11-09, `645af584`) was 99.98% (mypy) and
99.96% (rich). Wrong targets then: 23 and 8.

Verdict, Gate 3: no drop in the correct-target rate, no new wrong target. Still met.
Both `scippy index` runs exit with code 3 (finished, bounded references; see README, "The result of
`scippy index`"). Run time and memory are no measure: other agents shared the machine. The two mypy
runs ran together; the first took 29:59 and peaked at 6.26 GB. The second ran alone: 15:33.

### Loci comparison

The comparison uses the real `~/Notes` vault (read-only) and its roots. Scippy side: the
`relations` example of `scippy-loci` (release build of `554f5321`). Loci side: loci-core `a076136`,
`loci --json graph/relations`. A record is keyed by source, span, and kind. The vault changed
since 2026-11-09 (notes and roots grew).

| Date | Relations (loci-core / Scippy) | Disagreements |
|---|---|---|
| 2026-11-09 (saved) | 35,333 / 35,333 | 0 |
| 2026-11-16, `554f5321` | 61,261 / 61,261 | 0 |

Kinds: wikilink 9,597, embed 211, frontmatter_link 17,402, markdown_link 23,603, markdown_embed 13,
tag 10,435. Verdict, Gate 5: still met, now rerun after `04caacd`.

## Run 2026-11-21: graph contraction and return annotations

Fork commit 3240d66 on cda555a; Scippy on main 06df48a. The fork pin moves over two changes.

- Contraction: the builder simplifies the graph of each file at the end of a build. It prunes the
  scope nodes that lead nowhere. It merges each chain of plain scope nodes that have one incoming
  and one outgoing edge, and it keeps the precedence of the first edge. `set_contract_graph(false)`
  turns it off. Scippy leaves it on.
- Return annotations (annot2): a call of an annotated function follows the return annotation only.
  The value of the body stays when the annotation names no class: `Self`, `Any`, `Type[...]`, a
  `TypeVar`, `ParamSpec`, or alias that the same file binds, a type parameter, or a string.

Setup: mypy (187 files) scored against scip-python 0.6.8. Before is the pin cda555a. After is the
pin 3240d66. Both ran with `--trace`. The two changes also ran alone, to split the effects.

| mypy | Before | Contraction alone | annot2 alone | Both |
|---|---|---|---|---|
| Wrong targets | 0 | 0 | 0 | 0 |
| Raw recall | 94.11% | 94.11% | 94.19% | 94.19% |
| Recall with plausible targets | 98.31% | 98.33% | 98.48% | 98.48% |
| Bounded references | 330 | 296 | 0 | 0 |
| Wall time | 1,515 s | 1,290 s | 710 s | 533 s |
| Peak memory | 5.45 GB | 4.21 GB | 5.51 GB | 4.22 GB |

Contraction alone cuts the arena cells of the file phase from 83.4M to 37.4M and the peak memory
by 1.24 GB. Its Scippy output changes in 47 locations of mypy. 33 of them were bounded references
that now resolve. In 2 locations (`meet.py:617` and `:625`) Scippy reports a superset of targets that
still holds the answer of scip-python.

annot2 alone changes 381 locations of mypy. About 143 are gains that scip-python confirms. 9 are
losses where scip-python agrees with the old answer. A known shadowing issue causes them. A
separate investigation looks into it.

| Corpus | Contraction alone | annot2 alone |
|---|---|---|
| rich | 0 changed locations | 7 changed, 0 losses |
| stdlib | 0 changed locations | 1 changed |

Contraction applies to every language of the fork, so the TypeScript pack builds smaller graphs
too. Its golden and digest tests pass with no change to the golden files. The gate (`testee verify`,
full) is green on the commit of the bump.

## Run 2026-11-21: isinstance guards and the shadowing fix

Fork commit 930f3e2 (on 861db99, after the pin 3240d66); Scippy on main 6b42895. Baseline is the
pin 3240d66. Scored against scip-python.

The fork changes three things:

- It narrows more `isinstance` guards: `assert` with `and` (both operands), a parenthesized `if` or
  `elif` condition with `and`, a conditional expression, and `not isinstance(...) or ...` as the
  right operand and as an early-exit `if`.
- A string annotation that holds one plain name follows the annotation only. A string that holds
  more, such as `"Foo | None"`, keeps the body. The gate for annotations without a class accepts a
  name that ends in `TypeAlias`, such as `_TypeAlias`.
- The stitcher keeps one more similar path for the reference at an attribute store (syntax type
  `store`), when the known path is shadowable and the bucket holds fewer than 2 paths. This fixes
  the shadowing that caused the 9 losses of the previous run.

| Corpus | rich | mypy |
|---|---|---|
| Raw recall | 96.78% to 96.80% | 94.19% to 94.24% |
| Recall with plausible targets | 97.70% to 97.72% | 98.48% to 98.58% |
| Precision | 99.06%, unchanged | |
| Wrong targets | 0 | 0 |
| Bounded references | 4 to 0 | 0 |
| Changed locations | 5, no losses | 198, 0 resolved to unresolved |
| Engine work | 17% lower | |

On mypy, 21 of the 27 locations that 3240d66 lost resolve again. This includes all 9 that
scip-python confirms. One cost: 3 locations go from resolved to ambiguous (`server/update.py`
lines 1129, 1135, and 1137). A guard with a tuple of classes gives both classes.

Known gaps: `collections.abc` annotations, and `isinstance(x, TUPLE_CONSTANT)`.

## Run 2026-11-21: fork pin 27e2435 and the scope depth cap

Fork commit 27e2435 on 930f3e2; Scippy lane `pin-27e2435` on main e11c4b8. Baseline is the pin
930f3e2. Scored against scip-python 0.6.8, graph cache off, `-j 8`, `--trace`. The machine was
shared with other work (load average 2 to 10), so the times are indicative.

The fork changes three things:

- 37ef2f3: a vendored `tree-sitter-graph` with the syntax node id widened to `usize`. It fixes a
  flaky "Duplicate variable" failure under load.
- ac941ed: a `collections.abc` stub, and `isinstance` narrowing through a name bound to a tuple of
  classes.
- 27e2435: a method joins the class `member_attrs` only if its body can store an attribute of its
  first parameter. New stitcher option `with_max_scope_depth`, off by default.

Scippy sets the option for the Python pack only: `Some(1)` in `languages/python/src/rules.rs`, read
into the resolve step. The indexing step and the TypeScript pack are unchanged.

| Corpus | rich | mypy | stdlib |
|---|---|---|---|
| Raw recall | 96.80% to 96.82% | 94.24% to 94.24% | 97.72% to 97.73% |
| Recall with plausible targets | 97.72% to 97.74% | 98.58% to 98.62% | 98.29% to 98.30% |
| Precision (raw) | 99.06%, unchanged | 95.60% to 95.56% | 99.20%, unchanged |
| Wrong targets | 0 to 0 | 0 to 0 | 131 to 131 |
| Bounded references | 0 to 0 | 0 to 0 | 26 to 8 |
| Wall time, paired, `-j 8` | 24.4 s to 16.0 s | 290 s to 168 s | 170 s to 120 s |
| Peak memory | 0.57 GB to 0.57 GB | 4.16 GB to 4.26 GB | 1.58 GB to 1.61 GB |

The wall times come from paired runs of both binaries in one session. The older runs of the
baseline (22.9 s, 401.6 s, 789.5 s) ran under a different load and do not compare with the new
runs.

Changed locations, by transition (counts of locations; scip-python agreement in brackets):

| Transition | rich | mypy | stdlib |
|---|---|---|---|
| Changed locations | 6 of 27,046 | 687 of 188,825 | 62 of 88,831 |
| Unresolved to resolved | 2 (1 external agrees, 1 no target) | 616 (529 agree with the new answer, 77 have no scip-python target, 10 agree with neither) | 2 (neither) |
| Ambiguous to resolved | 4 (scip-python agrees with the new answer) | 57 (none agree) | 3 (agree with the new answer) |
| Bounded to resolved or other | 0 | 0 | 25 (13 resolved, 7 still bounded by work, 5 unresolved) |
| Resolved to ambiguous | 0 | 4 (scip-python agrees with the old answer, external) | 0 |
| Resolved to unresolved | 0 | 2 (scip-python agrees with neither) | 0 |
| Other | 0 | 8 | 32 (label change only: no-path to no-declaration) |

The 2 locations of mypy that go from resolved to unresolved are `semanal_newtype.py:139:27` and
`semanal_typeddict.py:492:17` (`x.line = ...`). The old answer was the three `line` members of
`Context`; scip-python gives the class `Context`. The 4 resolved-to-ambiguous locations are
`stubdoc.py:472`, `:475`, `:477`, and `:514`: a `list` method now also finds a `typing` stub, and
scip-python agrees with the old answer. This is the literal-versus-annotation ambiguity. The
unresolved-to-resolved locations of mypy are mostly `collections.abc` imports and annotations
(`Iterator`, `Sequence`).

The depth cap alone (the same pin without it, paired): no change in the final output on rich,
mypy, or stdlib. On stdlib, bounded references go from 10 to 8: 2 end as not targetable, and 6
change from the queue limit to the work limit. The earlier scratch measurement gave 2 bounded
with depth 1. The lane build does not reproduce that: the 6 references in `email/message.py` still
hit the work limit. The depth cap does not lower the wall time (stdlib 110 s without it, 120 s with
it; mypy 169 s and 168 s).

Known gaps: 8 bounded references on the standard library at the default limits (10 without the
depth cap). Cross-file `TypeVar`s. Literal-versus-annotation ambiguity in 7 mypy locations
(`server/update.py:1129`, `1135`, `1137`, and `stubdoc.py:472`, `475`, `477`, `514`).

The gate (`projectman verify`, full) is green on the lane. No TypeScript golden file changed.

## Run 2026-11-21: resolve threads over frozen partial paths

Fork commit dc75eb5 on c8e54f0; Scippy lane `resolve-threads` on main 6c0c0d3. The baseline is the
saved run of Scippy 6f43c300 on the pin 27e2435 (`pin-27e2435/runs`). Scored against scip-python
0.6.8, graph cache off, `-j 8` on 8 logical cores, `--trace`. The machine was shared with other
work (load average 3 to 8), so the times are indicative.

The fork commit adds `PartialPaths::freeze` and `FrozenPartialPaths::overlay`. An overlay reads the
frozen arenas and adds its new cells to its own storage. Scippy resolves the references of a shard
on threads. Each thread owns one overlay, takes the references in chunks of 16, and writes each
result to the slot of its reference. The merge reads the slots in order. The new option
`--resolve-threads N` sets the number of threads. The default is `--jobs` divided by the number of
shards that run at once, at least 1. Each corpus below is one shard, so the default is 8 threads.

The fork commits between the pin 27e2435 of the baseline and dc75eb5 are 349fb5d (shadow check in
one pass), c8e54f0 (cached regular expression of `replace`), and dc75eb5 (the overlay arenas).

| Corpus | rich | stdlib | mypy |
|---|---|---|---|
| Wall time, 1 thread | 13.1 s | 112.1 s | 189.9 s |
| Wall time, default (8 threads) | 10.7 s | 59.7 s | 95.9 s |
| Peak memory, 1 thread to 8 threads | 0.57 to 0.59 GB | 1.59 to 2.33 GB | 4.20 to 4.16 GB |
| `index.scip` at 1 and 8 threads | identical | identical | identical |
| `--trace` at 1 and 8 threads | identical | identical | identical |
| Output against the baseline run of 6f43c300 | identical | identical | identical |
| Raw recall | 96.82% | 97.73% | 94.24% |
| Precision (raw) | 99.06% | 99.20% | 95.56% |
| Wrong targets | 0 | 131 | 0 |
| Changed locations against the baseline | 0 | 0 | 0 |

The `index.scip` and `--trace` files of the 8-thread run have the same SHA-256 as the files of the
saved baseline run, so the two fork commits between the pins (349fb5d, c8e54f0) change no result on
these corpora. The oracle metrics equal the baseline metrics line by line. A run of the baseline in
the same session was not repeated: the baseline wall times in `pin-27e2435` ran under another load.

The speed-up is 1.2x on rich (10 s of work), 1.9x on stdlib, and 2.0x on mypy. The machine had
8 cores and a load of 3 to 8, so the figures understate an idle machine. The peak memory of stdlib
rises by 0.75 GB. The cause is not measured. The likely cause is that eight searches run at once,
and each one holds its own arena cells. The unit test
`resolving_on_threads_gives_the_bindings_of_one_thread` checks 1, 2, 4, and 7 threads on 40 files.

## Run 2026-11-21: build threads

Scippy lane `parbuild` on main f124952 (fork pin dc75eb5, no fork change). The baseline is the
release build of f124952. Graph cache off unless it is named, default `--jobs` (8 logical cores),
`--trace`. The machine was shared with other work (load average 5 to 10), so the times are
indicative. One run for each cell.

`--build-threads N` builds the graphs of the files of one shard on `N` threads. A file builds
without any other file. Workers parse the files and build each graph in a stack graph of its own.
The main thread looks the files up in the graph cache, adds the graphs to the engine in file
order, and stores the new graphs in file order. The default is the rule of `--resolve-threads`,
so each corpus below (one shard) builds on 8 threads. The compiled TSG rules are shared:
`StackGraphLanguage` is `Send` and `Sync`, so no fork change was needed.

| Corpus | rich | stdlib | mypy |
|---|---|---|---|
| Wall time, baseline | 10.9 s | 57.4 s | 94.3 s |
| Wall time, default (8 build threads) | 5.1 s | 37.6 s | 43.1 s |
| Wall time, `--build-threads 1` | 11.4 s | 61.6 s | 101.2 s |
| Peak memory, baseline | 0.55 GiB | 2.20 GiB | 4.01 GiB |
| Peak memory, default | 1.25 GiB | 3.35 GiB | 7.67 GiB |
| Peak memory, `--build-threads 1` | 0.52 GiB | 2.23 GiB | 3.91 GiB |
| `index.scip`, default and `--build-threads 1`, against the baseline | identical | identical | identical |
| `--trace`, default and `--build-threads 1`, against the baseline | identical | identical | identical |

The SHA-256 prefixes of the files are `73da72b5357a1443` (rich `.scip`), `470020b42fff4add`
(stdlib `.scip`), and `ec5574352ad506bd` (mypy `.scip`). The stdlib run exits with 3 (incomplete
input) in the baseline and in the new build alike.

Graph cache on mypy, default threads, one run each: cold 52.3 s and 8.02 GiB (187 files built),
warm 38.5 s and 5.18 GiB (187 files from the cache). The `.scip` and `--trace` files of both runs
are identical to the baseline files. With the cache on, the main thread holds at most 256 graphs
from the cache before it adds them.

Earlier measurements of the prototype, with the graph cache off and 8 resolve threads on mypy:
4 build threads 49 s and 6.7 GiB, 8 build threads 45 s and 7.8 GiB, against 94 s and 4.0 GiB for
a serial build. The memory estimate of a shard (`shard_estimate`) rises for the build threads:
the growing part was 1.25, 1.75, and 2.0 times for 2, 4, and 8 threads or more (see the next section for the current values). The measured ratios
of the peak are 1.21, 1.68, and 1.95.

The engine now builds each file in a stack graph of its own and loads the result, also with one
thread. The loaded arena differs from the arena of a direct build, so the search cost
`arena_cells_max` rises by one cell in two golden fixtures. The `.scip` and `--trace` files do not
change. The serial build takes 5% to 8% longer (one run each, under load). The cause is the copy of each graph.

Tests (`crates/scippy-core/tests/build_threads.rs`, `crates/scippy-cli/tests/build_threads.rs`):
identical output and report at 1, 2, 3, 8, 64 threads and the default; cold, warm, mixed, and
serial-after-parallel cache runs equal a run without the cache; five files that fail to build
(a rule error, a second dialect whose rules do not compile, a syntax error) give the same
diagnostics and the same output at 1 and 8 threads; a panic in a worker ends the run and writes
no file.

## Run 2026-11-22: new baseline with named numbers

Date: 2026-11-22. Scippy `df9cecaa` (fork pin `dc75eb5`), scip-python 0.6.8. No indexer ran. The
scorer (`score.py`, `analyze.py`, `metrics.py`) read stored indexes:

- mypy, rich, stdlib: Scippy index and trace of `~/.local/state/scippy/pin-27e2435/runs/`
  (byte-identical to the output of `df9cecaa`); scip-python index of the earlier score runs.
- scrapy, celery, pydantic-ai: Scippy index and trace of the `corpora` run of 2026-11-21
  (Scippy `df9cecaa`, default limits). The pydantic-ai reference index is the one of 2026-11-15,
  made with the pydantic environment.
- Corpora: mypy 1.19.1, rich `3a109cc`, stdlib (CPython 3.14.7, 160 files), scrapy 2.6.3,
  celery 5.3.6, pydantic-ai `ca52326`. The layout and the fetch steps are in `README.md`.
  The baseline run is `df9cecaa-2026-11-22` in `classes.csv`.

Scorer changes in this run:

1. `sp_class_path` reads a top-level module without backticks (`dataclasses/Field#`). Before, the
   owner-class rule missed these symbols, and the positions counted as wrong.
2. New class `sp_first_binding`: scip-python takes the first of two declarations of a name in one
   scope, and Scippy a later live one.
3. New class `sp_builtin_type_attr`: scip-python gives `builtins/type#` for an attribute named
   `type`. It counts on its own. Earlier it sat in `scip_python_missing_or_wrong`.
4. `metrics.py` reports named numbers, with the old raw numbers next to them (see `README.md`).

Named numbers (counts are positions that both tools resolve in the project, except the last two rows):

| Number | mypy | rich | stdlib | scrapy | celery | pydantic-ai |
|---|---|---|---|---|---|---|
| positions of scip-python (`sp`) | 154,016 | 20,411 | 59,960 | 17,374 | 33,120 | 160,608 |
| positions of both (`both`) | 151,891 | 19,950 | 59,072 | 17,261 | 32,823 | 155,564 |
| strict recall | 94.24% | 96.82% | 97.73% | 96.64% | 97.53% | 95.99% |
| strict precision | 95.56% | 99.06% | 99.20% | 97.27% | 98.41% | 99.10% |
| owner-class coarse | 4,724 | 79 | 364 | 13 | 118 | 8 |
| import-prefix convention | 1,812 | 109 | 2 | 457 | 379 | 1,140 |
| same-name other class | 206 | 0 | 30 | 0 | 7 | 211 |
| function type parameter | 0 | 0 | 6 | 0 | 0 | 0 |
| `sp_first_binding` | 0 | 0 | 62 | 1 | 0 | 0 |
| real wrong | 0 | 0 | 10 | 0 | 17 | 44 |
| `sp_builtin_type_attr` (Scippy has a target, scip-python a builtin) | 1,327 | 11 | 12 | 2 | 9 | 31 |
| Scippy missing (`sp` has a target, Scippy none) | 2,125 | 461 | 888 | 113 | 297 | 5,044 |

Old raw numbers, for continuity. Overlap recall and precision equal the strict numbers on every
corpus. The old "wrong" counted real wrong plus `sp_first_binding`.

| Old number | mypy | rich | stdlib | scrapy | celery | pydantic-ai |
|---|---|---|---|---|---|---|
| raw recall | 94.24% | 96.82% | 97.73% | 96.64% | 97.53% | 95.99% |
| raw precision | 95.56% | 99.06% | 99.20% | 97.27% | 98.41% | 99.10% |
| wrong, old definition | 0 | 0 | 72 | 1 | 17 | 44 |
| wrong, before this run (older scorer) | 0 | 0 | 131 | 1 | 17 | 44 |

Stdlib wrong targets fall from 131 to 10. 59 positions were the regex bug (they are now owner-class
or same-name positions). 62 are `sp_first_binding`: `enum.py` 59 (`Flag` 32, `Enum` 11, `ReprEnum` 6,
`_not_given` 5, `_auto_null` 5), `asyncio/futures.py` 2, `functools.py` 1. The 10 that remain:

- `asyncio/tasks.py:840`: a real Scippy error (a read after `nonlocal x; x += 1`);
- `functools.py:213-221`, 5 positions: the attribute `obj` of a class that a function defines
  (`cmp_to_key().K`); scip-python gives `K#`, Scippy gives the local `obj` (the review counted
  these as "member of a function-local class", not as a Scippy bug);
- `importlib/util.py:186, 189, 222`, 3 positions: `__class__` in a method; scip-python is wrong
  (it picks the method, Scippy picks the local `__class__`);
- `asyncio/events.py:797`: `from .unix_events import _DefaultEventLoopPolicy` in the `else` branch;
  scip-python gives the `windows_events` target of the `if` branch. This is a first-binding error of
  scip-python too, in an import in another file, which the rule does not cover.

Other changes against the older numbers: rich, mypy, scrapy, celery, and pydantic-ai keep their
numbers, except that the `type` attribute positions now have their own class (mypy 1,327 of the
1,374 positions of scip-python that point at `builtins/type#`; the other 47 are names named `type`).
Scrapy's one wrong target was `deco = deco(use_instead)`: it is now `sp_first_binding`. The 17 wrong
targets of celery are the `X = X` pattern (`Timer = Timer` in a class body). They stay real wrong.

## Run 2026-11-22: build window, no scope depth cap

Scippy lane `bundle` on main 9485b036 (fork pin dc75eb5). Graph cache off, default `--jobs` (8
logical cores), `--trace`, GNU time. One run for each cell. The machine was shared (load average 4
to 20 before each run), so the wall times are not comparable. The peak memory does not depend much
on the load.

Changes: the lookup of the build threads runs at most `max(16, 4 * threads)` files ahead of the
merge, and the Python rules set no scope depth cap (`max_scope_depth: None`; it was `Some(1)`).

| Peak memory | rich | stdlib | mypy |
|---|---|---|---|
| `--build-threads 1` (`df9cecaa`) | 0.52 GiB | 2.23 GiB | 3.91 GiB |
| 2 build threads | | | 4.36 GiB |
| 4 build threads | | | 4.70 GiB |
| default (8 build threads), before | 1.25 GiB | 3.35 GiB | 7.67 GiB |
| default (8 build threads), now | 1.03 GiB | 2.81 GiB | 5.47 GiB |

The ratio of mypy to its 1-thread peak is 1.12, 1.20, and 1.40 for 2, 4, and 8 threads. The
estimate of a shard now rises up to 1.5 times (before: 2 times).

Output against `df9cecaa` (the saved files of the run 2026-11-21): the `.scip` files of the three
corpora differ in one byte only, the version text `0.2.0` against `0.1.0` in the tool info. The
`--trace` files of rich and mypy are identical. The `--trace` file of stdlib differs in 242 of
88,832 lines: 234 resolved references and 6 bounded references with the same targets and other
counts of `ends` and `shadowed`, and 2 references that move from `not-targetable` to `bounded`.
A control run with the cap restored (`Some(1)`) gives the exact trace of `df9cecaa` on stdlib
(SHA-256 prefix `2eea08994d584eaf`), so the scope depth cap is the only cause.

On stdlib, removing the cap changes 242 trace lines and moves 2 references (`logging/config.py:606` and `:619`) from not-targetable to bounded. Bounded references rise from 8 to 10 (3 by work and 7 by queue length, against 7 by work and 1 by queue length before). The `.scip` file is identical.

## Run 2026-11-22: fork rules, kind A identity merge, function rule

Scippy lane `integrate` on main fb122d80. Fork pin `e945e1d` (baseline `df9cecaa`: `4e20bf8`). Label
`integrate-ebfc6892` in `classes.csv`. Baseline `df9cecaa-2026-11-22`. Graph cache off, `--trace`, all six
corpora in parallel. Correctness only: no timing was taken.

Changes in this run:

- The fork rules of the A1 to A6 bundle (rules2) and of the base class order, `super()`, and the
  guard of an attribute (rules3, `325bf62`, `11fc107`).
- Kind A: a variable or a field that rebinds a class, a function, or a property of its own container
  shares its symbol (`Future = _asyncio.Future` after `class Future`; `self.plain = v` next to a
  property `plain`). The symbol keeps the kind of the class or the function.
- Kind F: for a plain reference, the candidate that a function holds (its scope chain is a prefix of
  the enclosing scopes of the reference and ends in a function) wins over the candidates of an outer
  container. The fork rules cannot hoist the binding (`e945e1d`, `test/hoisting.py.skip`).
- `max_scope_depth` is gone, and `anchor_from_info` takes the source info by reference.

Named numbers, before (`df9cecaa`) and after:

| corpus | both | scippy_missing | real wrong | owner-class coarse | same-name other class | strict recall | strict precision |
|---|---|---|---|---|---|---|---|
| mypy | 151891 to 151916 | 2125 to 2100 | 0 to 0 | 4724 to 4724 | 206 to 206 | 94.2428 to 94.2590 | 95.5613 to 95.5620 |
| rich | 19950 to 20009 | 461 to 402 | 0 to 0 | 79 to 79 | 0 to 0 | 96.8203 to 97.1094 | 99.0576 to 99.0604 |
| stdlib | 59072 to 59188 | 888 to 772 | 10 to 9 | 364 to 364 | 30 to 28 | 97.7285 to 98.0304 | 99.1976 to 99.3090 |
| scrapy | 17261 to 17290 | 113 to 84 | 0 to 0 | 13 to 13 | 0 to 0 | 96.6387 to 96.8113 | 97.2713 to 97.2817 |
| celery | 32823 to 32906 | 297 to 214 | 17 to 2 | 118 to 120 | 7 to 6 | 97.5302 to 97.8231 | 98.4127 to 98.4592 |
| pydantic-ai | 155564 to 155651 | 5044 to 4957 | 44 to 44 | 8 to 8 | 211 to 211 | 95.9859 to 96.0400 | 99.0981 to 99.0986 |

Ambiguous references (`--trace` status `ambiguous`), and bounded references:

| corpus | ambiguous before | ambiguous after | bounded before | bounded after |
|---|---|---|---|---|
| mypy | 339 | 313 | 0 | 0 |
| rich | 18 | 1 | 0 | 0 |
| stdlib | 155 | 88 | 8 | 10 |
| scrapy | 12 | 5 | 0 | 0 |
| celery | 58 | 24 | 17 | 18 |
| pydantic-ai | 175 | 149 | 0 | 0 |

Changed locations against the baseline (`difftool.py`), sorted:

- Gained locations (the baseline had no target): rich 50, mypy 30, stdlib 146, scrapy 37, celery 89,
  pydantic-ai 113. Of these, 177 were ambiguous in the baseline (rich 17, mypy 26, stdlib 67, scrapy 7,
  celery 34, pydantic-ai 26). Three gained locations
  differ from scip-python (`asyncio/proactor_events.py:621:16`, `celery/worker/request.py:170:13` and
  `:41`); the Scippy target (the field of the class) is the right one in each.
- Intended gain or neutral, kind A (a target that widens to the definition sites of the merged
  symbol): rich 55, stdlib 101, scrapy 1, celery 11, pydantic-ai 16. Scip-python agrees with the new
  target or with both in all positions that it resolves in the project.
- Neutral, `super().__init__()` (rules3 `325bf62`): the old target was the builtin class `super`,
  the new one is `object.__init__`. Both are external, and neither is the `__init__` of the base
  class that scip-python gives where the base is generic or from the standard library. Count: rich 5,
  mypy 9, stdlib 13, scrapy 15, celery 21, pydantic-ai 59 (all in the allowlist).
- Allowed from the bundle (rules2): the `X = X` locations of celery and `asyncio/tasks.py:840`.
- Known regression, fix in progress (`object` after all explicit bases). One location: `argparse.py:1887:48`, `super(ArgumentParser, self).__init__` with the
  bases `(_AttributeHolder, _ActionsContainer)`. The baseline target was
  `_ActionsContainer.__init__` (scip-python agrees). Now it is `object.__init__`: the search of the
  base classes is depth first from the left, and the implicit `object` of `_AttributeHolder` comes
  before the second base. The cause is `325bf62` of the fork (not the C3 order of Python). It is in
  the allowlist with this reason.
- Bounded by work, known (rules2): celery `celery/app/builtins.py:92:25`. Stdlib `logging/config.py`
  606 and 619 go from not targetable to bounded.

## Run 2026-11-22: object after all bases (fork `32b5325`)

Scippy lane `objlast` on main `f1e60474`. Fork pin `32b5325` (before: `e945e1d`). Label
`objlast-32b5325` in `classes.csv`. Reference run: `integrate-ebfc6892`. Graph cache off, `--trace`, all six
corpora in parallel. Correctness only: no timing was taken. The two celery locations of "Open" are in the allowlist.

Changes of the fork (`f8757f5`, `32b5325`):

- A base named `object` is no edge of the super scope. The path through the first base had the higher
  precedence, so `object.__init__` came before the second base. Every class now reaches `object` from its
  self scope, with a precedence below its base classes. A base class adds no `object` of its own.
- The call of `super()` follows a generic base class, `class B(A[T])`, in the order of the bases.
  Before, it fell back to `object.__init__`. A lookup of a member still does not follow a generic base.
- Tests: `test/object_last.py` (the argparse shape, a generic base, a diamond, a single base) and five
  tests in `rust/builtins.rs`. They fail on the old rules. The pin of `e945e1d` shows the first problem
  in `argparse.py:1887:48`.

Named numbers, `integrate-ebfc6892` to `objlast-32b5325`:

| corpus | both | scippy_missing | real wrong | owner-class coarse | same-name other class | strict recall | strict precision |
|---|---|---|---|---|---|---|---|
| mypy | 151916 to 151918 | 2100 to 2098 | 0 to 0 | 4724 to 4724 | 206 to 206 | 94.2590 to 94.2603 | 95.5620 to 95.5621 |
| rich | 20009 to 20009 | 402 to 402 | 0 to 0 | 79 to 79 | 0 to 0 | 97.1094 to 97.1094 | 99.0604 to 99.0604 |
| stdlib | 59188 to 59189 | 772 to 771 | 9 to 9 | 364 to 364 | 28 to 28 | 98.0304 to 98.0320 | 99.3090 to 99.3090 |
| scrapy | 17290 to 17290 | 84 to 84 | 0 to 0 | 13 to 13 | 0 to 0 | 96.8113 to 96.8113 | 97.2817 to 97.2817 |
| celery | 32906 to 32906 | 214 to 214 | 2 to 2 | 120 to 120 | 6 to 6 | 97.8231 to 97.8231 | 98.4592 to 98.4592 |
| pydantic-ai | 155651 to 155961 | 4957 to 4647 | 44 to 44 | 8 to 8 | 211 to 211 | 96.0400 to 96.2331 | 99.0986 to 99.1004 |

Ambiguous references: celery 24 to 26, all others equal. Bounded references: no change.

Changed locations against the integrate run (`difftool.py`):

- `argparse.py:1887:48` is `_ActionsContainer.__init__` again (scip-python agrees). It leaves the allowlist.
- 34 `super().__init__()` locations change to a project target where scip-python agrees with the new
  target (`sp=cand_only`): pydantic-ai 31, mypy 2 (`mypy/typeops.py:1123:16`, `mypy/types.py:4030:16`), stdlib 1
  (argparse). They are single or first generic bases (`Model[AsyncClient]`). The other 5 changes are
  external on both sides.
- Gained locations (the integrate run had no target): pydantic-ai 420 (the keyword names and the
  arguments of the `super().__init__()` calls, and members of `object`), stdlib 164, celery 40, and
  fewer in the other corpora. The members of `object` (`self.__dict__`) resolve for every class now. The
  `builtins.different` numbers rise because scip-python writes the symbol of such a member in another
  form (`__dict__#` against `object#__dict__.`).
- No new real wrong target on any corpus.

Open: two locations of celery change from resolved to ambiguous against the integrate run:
`celery/canvas.py:1352:35` and `:1354:23`, `super().__new__` in `chain(_chain)` with `Signature(dict)`.
The targets are now `dict.__new__` and `object.__new__`. Scip-python gives an external symbol. Cause: a
method of a base class that stores `self.x` leads back to the self scope of that base through the member
attributes, and a classmethod that stores `cls.x` leads to the members. Two different routes, so the
route to `object` is not shadowed by the route to `dict`. The gate fails on these two locations. They
are in the allowlist (R4) and the lane is landed. They have the same structure as kind D: a self or
cls store route re-enters the class self scope, so routes that diverge early cannot shadow each other.
The engine design spike on precedence over whole paths will cover it.


## Run 2026-11-22: unions, one occurrence for each candidate

Scippy lane `unions` on main `ccbe8688`. Fork pin `32b5325` (unchanged). Label `unions-32b5325` in
`classes.csv`. Base run: `objlast-32b5325`. Graph cache off, `--trace`, all six corpora in parallel.
Correctness only: no timing was taken. Gate against `objlast-32b5325`: pass on all six corpora.

Change: when a reference is ambiguous, and every candidate is a method, an attribute, or a property of
a project class, and there are at most 4 candidates, and no owner class is one class with another owner
or a subclass or superclass of it (transitive), the index has one occurrence for each candidate at one
range, sorted by symbol. Everything else is as before: override chains (kinds B and D), module-level
names, mixed kinds, members of external classes (for example `BaseExceptionGroup` and
`ExceptionGroup`), and more than 4 candidates.

New oracle numbers: exact is the old strict number, now with the rule "Scippy has one symbol". Covered
adds the multi-symbol positions whose set holds the scip-python target. A multi-symbol position is
never exact, and it is not in `both`. The gate prints the count against the baseline, and fails on a
set with an unrelated-owner violation (see `owners.py`).

| corpus | multi-symbol | with sp target | covered | not covered (owner-class coarse) | sp external | exact recall | covered recall | exact precision | covered precision |
|---|---|---|---|---|---|---|---|---|---|
| mypy | 194 | 176 | 122 | 54 (49) | 18 | 94.2603 | 94.3395 | 95.5621 | 95.5317 |
| rich | 0 | 0 | 0 | 0 | 0 | 97.1094 | 97.1094 | 99.0604 | 99.0604 |
| stdlib | 10 | 7 | 7 | 0 | 3 | 98.0320 | 98.0437 | 99.3090 | 99.3091 |
| scrapy | 1 | 1 | 1 | 0 | 0 | 96.8113 | 96.8171 | 97.2817 | 97.2818 |
| celery | 15 | 15 | 15 | 0 | 0 | 97.8231 | 97.8684 | 98.4592 | 98.4599 |
| pydantic-ai | 110 | 110 | 109 | 1 | 0 | 96.2331 | 96.3009 | 99.1004 | 99.1004 |

The exact numbers equal `objlast-32b5325` to four decimals on every corpus. `scippy_missing` falls by
the number of multi-symbol positions that have a scip-python target (mypy 2098 to 1922, pydantic-ai
4647 to 4537, celery 214 to 199, stdlib 771 to 764, scrapy 84 to 83). Covered precision on mypy falls
by 0.03 pp: 176 positions join the denominator and 122 of them hold the target (54 do not).
Real wrong: unchanged on all corpora (mypy 0, rich 0, stdlib 9, scrapy 0, celery 2, pydantic-ai 44).

Ambiguous references (`references ambiguous` is the outcome of the reconciler and still counts a
union; `references union` is a part of it):

| corpus | ambiguous before | union | ambiguous left (no occurrence) |
|---|---|---|---|
| mypy | 313 | 194 | 119 |
| rich | 1 | 0 | 1 |
| stdlib | 88 | 10 | 78 |
| scrapy | 5 | 1 | 4 |
| celery | 26 | 15 | 11 |
| pydantic-ai | 149 | 110 | 39 |

The estimate of the task (mypy about 145, stdlib about 27, rich 1) was too low for mypy and too high
for stdlib and rich. Cause: the stdlib and rich sets are mostly members of external classes, or have
related owners (override chains).

Review of the 330 new multi-symbol positions with `difftool.py` (its `--out` writes
`PREFIX.multi.tsv`): lost 0 and changed 0 on every corpus, gained = the multi-symbol positions. The
gate found 0 sets with related owners, 0 sets with a module-level symbol, and 0 sets of more than 4
symbols (the inheritance comes from the corpus sources and from both indexes). A sample of each group
by hand:

- mypy: `ListExpr` or `TupleExpr` `.items` (27 positions), `ImportAll` or `ImportFrom` `.id`,
  `CallableType` or `Overloaded` `.with_unpacked_kwargs()`, `FuncDef` or `OverloadedFuncDef` `.name()`.
  Every pair is a pair of sibling classes. `Decorator` and `FuncBase` are no subclasses of each other.
- pydantic-ai: `ModelResponse` or `StreamedResponse` `.metadata` (13), the four `FileUrl` subclasses
  `.url`, `ApprovalRequired` or `CallDeferred` `.metadata`. Sibling classes.
- celery: 11 of 15 are `_task_stack.push`, `.pop`, `.top`, `.stack`, where `LocalStack` is
  `_FastLocalStack` or `_LocalStack` by a flag (`USE_FAST_LOCALS`). It is a conditional class alias. The
  two classes are unrelated, so the set is a union of types, and scip-python agrees with one of them.
- stdlib: `_ModuleLock` or `_DummyModuleLock` `.acquire()`, `minidom.Document` or `Element`
  `.childNodes`.
- Not covered: 49 of 54 on mypy are owner-class coarse (scip-python gives the class). The other 5 on
  mypy and the 1 on pydantic-ai: scip-python picks the declaration of a base class (`FunctionLike#fallback`,
  `Node#accept()`, `FuncBase#info`, `BaseToolReturnPart#content`), and Scippy has the members of the
  subclasses. They are kind D questions of the engine design spike, not wrong sets.
- 18 positions on mypy and 3 on stdlib have a scip-python target outside the project (`x.type` is
  `builtins/type#` for scip-python). They are in `sp_builtin_type_attr` (mypy 1327 to 1345).

Consumers: `scippy metalayer` reads the occurrences of a file, the references with their candidates,
and `explain`. The store already keeps one row for each (range, symbol, role). The reference row stays
`ambiguous` with its candidates. Tests: `crates/scippy-cli/tests/union_query.rs` (index, `.scip`
occurrences, and `explain --to` for each candidate) and a metalayer store test. The SCIP specification
allows several occurrences at one range (see CHANGELOG).

Limit: a base class that the project does not resolve gives no relation, so two classes with a hidden
relation would count as unrelated. The gate cross-checks with the inheritance of scip-python and the
class statements of the sources. It found none.
