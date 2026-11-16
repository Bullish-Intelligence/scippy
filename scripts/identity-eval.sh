#!/usr/bin/env bash
# Evaluate entity rebinding. Not part of the gate. Run inside the devenv shell.
#
#   scripts/identity-eval.sh replay REPO_COPY WORK_DIR [PAIRS] [TIME_PAIRS] [MOVES]
#       Replay PAIRS consecutive commit pairs (default 20) of a COPIED git repository.
#       Print per-tier precision and recall, the orphan and ambiguity rates, and (for the first
#       TIME_PAIRS pairs, default 0) the overhead of the rebinding. MOVES (default 0) is the
#       number of pairs chosen from commits that rename files or add and delete files.
#       Time is CPU time, so machine load does not distort it.
#       WORK_DIR is a scratch folder under ~/.local/state/scippy/. Never use a real repository.
#   scripts/identity-eval.sh overhead [WORK_DIR]
#       Time the rebinding on a synthetic corpus. The devenv task `bench:identity` runs this.
#
# Set SCIPPY to use a prebuilt binary. Otherwise the script builds the release binary.
set -euo pipefail

cd "$(dirname "$0")/.."
here=$(pwd)

binary() {
  if [[ -n "${SCIPPY:-}" ]]; then
    printf '%s\n' "$SCIPPY"
    return
  fi
  local target="${CARGO_TARGET_DIR:-${DEVENV_STATE:-$here/target}/cargo-target}"
  CARGO_TARGET_DIR="$target" cargo build --release -p scippy-cli >&2
  printf '%s\n' "$target/release/scippy"
}

command=${1:-}
shift || true
case "$command" in
  replay)
    repo=${1:?usage: identity-eval.sh replay REPO_COPY WORK_DIR [PAIRS] [TIME_PAIRS] [MOVES]}
    work=${2:?missing WORK_DIR}
    mkdir -p "$work"
    exec python3 -I "$here/scripts/identity_eval.py" replay "$repo" --work "$work" \
      --scippy "$(binary)" --pairs "${3:-20}" --time-pairs "${4:-0}" --moves "${5:-0}"
    ;;
  overhead)
    work=${1:-${TMPDIR:-/tmp}}
    exec python3 -I "$here/scripts/identity_eval.py" overhead --scippy "$(binary)" --work "$work"
    ;;
  *)
    sed -n '2,15p' "$0" >&2
    exit 2
    ;;
esac
