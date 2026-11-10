# Shared settings of the oracle scripts. Source it; do not run it.
# ORACLE_DIR holds the corpora, scip-python, and env-empty.json.
ORACLE_DIR=${ORACLE_DIR:-$HOME/.local/state/scippy/oracle}
HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
