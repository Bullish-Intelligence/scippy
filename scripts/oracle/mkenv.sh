#!/usr/bin/env bash
# usage: mkenv.sh CORPUS
# Build a minimal Python environment for scip-python in ORACLE_ENVS/CORPUS (default ORACLE_DIR/envs).
# It installs pydantic, plus pydantic-settings, sqlmodel, and typing_extensions when the corpus
# imports them. It installs wheels only, so no build step runs. It never installs the corpus.
# It writes CORPUS/env.json in the format of `scip-python --environment`.
set -eu
. "$(dirname "$0")/env.sh"
C=$1
ROOT=$ORACLE_DIR/corpora/$C/root
E=$ORACLE_ENVS/$C
imports() { grep -rhqE --include='*.py' "^\s*(from|import) $1\b" "$ROOT"; }
PKGS=(pydantic)
imports pydantic_settings && PKGS+=(pydantic-settings)
imports sqlmodel && PKGS+=(sqlmodel)
imports typing_extensions && PKGS+=(typing_extensions)
mkdir -p "$E"
uv venv --quiet --clear "$E/venv"
uv pip install --quiet --python "$E/venv/bin/python" --only-binary :all: "${PKGS[@]}"
"$E/venv/bin/python" -I - > "$E/env.json" <<'PY'
import importlib.metadata as m, json
out = []
for d in m.distributions():
    fs = [str(f) for f in (d.files or []) if not str(f).startswith("..") and "__pycache__" not in str(f) and str(f).endswith((".py", ".pyi"))]
    out.append({"name": d.name, "version": d.version, "files": fs})
json.dump(out, __import__("sys").stdout)
PY
echo "$C: ${PKGS[*]}"
