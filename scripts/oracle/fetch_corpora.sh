#!/usr/bin/env bash
# usage: fetch_corpora.sh [NAME ...]      NAME is one of: mypy rich stdlib scrapy celery pydantic-ai
# Build the standard corpora in ORACLE_DIR/corpora/NAME/root from pinned sources. Without NAME it builds all.
# Each root holds only .py files (and `py.typed` for rich), so scores do not depend on other files.
# The script downloads archives to ORACLE_DIR/downloads/NAME (set ORACLE_DOWNLOADS to use another folder)
# and checks the SHA-256 of each archive. It extracts each archive in a new empty folder and never runs
# any code of a corpus. It does not touch a root that exists: delete the root to build it again.
#
# Pins (version, source, SHA-256 of the archive, or Git commit):
#   mypy         1.19.1   PyPI sdist mypy-1.19.1.tar.gz, package mypy/ (187 files; version.py is one line, see below)
#                         19d88bb05303fe63f71dd2c6270daca27cb9401c4ca8255fe50d1d920e0eb9ba
#   rich         3a109cc  https://github.com/Textualize/rich commit 3a109ccdffe5b2d0e30684a304fbd388e99ce57b, package rich/ (100 files)
#   stdlib       3.14.7   python.org Python-3.14.7.tar.xz, the 160 files of corpus-stdlib-files.txt from Lib/
#                         3b48dac8fb59f62eaa67ac83c1eb12bda1b7a08406dd286e252c11a66be27f81
#                         One line differs from the stored corpus: Nix patches the path of the shell in
#                         subprocess.py:1850. No reference of any tool is on that line.
#   scrapy       2.6.3    PyPI sdist Scrapy-2.6.3.tar.gz, package scrapy/ (171 files, mostly unannotated)
#                         bdff1aaf3547c2e540436d3ab8680b21039327bd5d3a2ef89c34165992d3e5f3
#   celery       5.3.6    PyPI sdist celery-5.3.6.tar.gz, package celery/ (155 files, dynamic patterns)
#                         870cc71d737c0200c397290d730344cc991d13a057534353d124c9380267aab9
#   pydantic-ai  ca52326  https://github.com/pydantic/pydantic-ai commit ca5232673b4dd7ce8475983fca304bd70ad6727d,
#                         folder pydantic_ai_slim/ (425 files)
#
# The scip-python reference indexes are not built here. They come from scip-python 0.6.8 (run_tools.sh) and
# live in ORACLE_DIR/ref/NAME.scip. See README.md.
set -eu
. "$(dirname "$0")/env.sh"
DL=${ORACLE_DOWNLOADS:-$ORACLE_DIR/downloads}
PYPI=https://files.pythonhosted.org/packages/source

# fetch NAME URL SHA256: download to DL/NAME/FILE unless it is there, check the hash, print the path.
fetch() {
  local name=$1 url=$2 sha=$3 file
  file=$DL/$name/$(basename "$url")
  mkdir -p "$DL/$name"
  [ -f "$file" ] || curl -fsSL --retry 3 -o "$file" "$url"
  echo "$sha  $file" | sha256sum -c --quiet - >&2 || { echo "$name: SHA-256 mismatch for $file" >&2; exit 1; }
  echo "$file"
}

# copy_py SRC DEST: copy the .py files of folder SRC, with their relative paths, into DEST.
copy_py() {
  (cd "$1" && find . -name '*.py' | sort) | while read -r f; do
    mkdir -p "$2/$(dirname "$f")"
    cp "$1/$f" "$2/$f"
  done
}

build() {
  local name=$1 root=$ORACLE_DIR/corpora/$1/root tmp tar x
  if [ -e "$root" ]; then echo "$name: $root exists. Skip."; return; fi
  mkdir -p "$ORACLE_DIR/corpora/$name"
  mkdir -p "$DL/$name"
  tmp=$(mktemp -d "$DL/$name/extract.XXXXXX")
  mkdir -p "$root.partial"
  case $name in
    mypy)
      tar=$(fetch mypy $PYPI/m/mypy/mypy-1.19.1.tar.gz 19d88bb05303fe63f71dd2c6270daca27cb9401c4ca8255fe50d1d920e0eb9ba)
      tar -xzf "$tar" -C "$tmp" mypy-1.19.1/mypy
      mkdir -p "$root.partial/mypy"; copy_py "$tmp/mypy-1.19.1/mypy" "$root.partial/mypy"
      # The standard corpus keeps a one-line version.py: the sdist file runs `git` code at import.
      echo '__version__ = "1.19.1"' > "$root.partial/mypy/version.py" ;;
    scrapy)
      tar=$(fetch scrapy $PYPI/S/Scrapy/Scrapy-2.6.3.tar.gz bdff1aaf3547c2e540436d3ab8680b21039327bd5d3a2ef89c34165992d3e5f3)
      tar -xzf "$tar" -C "$tmp" Scrapy-2.6.3/scrapy
      mkdir -p "$root.partial/scrapy"; copy_py "$tmp/Scrapy-2.6.3/scrapy" "$root.partial/scrapy" ;;
    celery)
      tar=$(fetch celery $PYPI/c/celery/celery-5.3.6.tar.gz 870cc71d737c0200c397290d730344cc991d13a057534353d124c9380267aab9)
      tar -xzf "$tar" -C "$tmp" celery-5.3.6/celery
      mkdir -p "$root.partial/celery"; copy_py "$tmp/celery-5.3.6/celery" "$root.partial/celery" ;;
    stdlib)
      tar=$(fetch stdlib https://www.python.org/ftp/python/3.14.7/Python-3.14.7.tar.xz 3b48dac8fb59f62eaa67ac83c1eb12bda1b7a08406dd286e252c11a66be27f81)
      tar -xJf "$tar" -C "$tmp" Python-3.14.7/Lib
      while read -r f; do mkdir -p "$root.partial/$(dirname "$f")"; cp "$tmp/Python-3.14.7/Lib/$f" "$root.partial/$f"; done < "$HERE/corpus-stdlib-files.txt" ;;
    rich)
      git clone --quiet https://github.com/Textualize/rich "$tmp/rich"
      git -C "$tmp/rich" -c advice.detachedHead=false checkout --quiet 3a109ccdffe5b2d0e30684a304fbd388e99ce57b
      mkdir -p "$root.partial/rich"; copy_py "$tmp/rich/rich" "$root.partial/rich"
      cp "$tmp/rich/rich/py.typed" "$root.partial/rich/" ;;
    pydantic-ai)
      git clone --quiet https://github.com/pydantic/pydantic-ai "$tmp/pydantic-ai"
      git -C "$tmp/pydantic-ai" -c advice.detachedHead=false checkout --quiet ca5232673b4dd7ce8475983fca304bd70ad6727d
      copy_py "$tmp/pydantic-ai/pydantic_ai_slim" "$root.partial" ;;
    *) echo "unknown corpus: $name" >&2; exit 2 ;;
  esac
  mv "$root.partial" "$root"
  rm -rf "$tmp"
  echo "$name: $(find "$root" -name '*.py' | wc -l) .py files in $root"
}

if [ $# -eq 0 ]; then set -- mypy rich stdlib scrapy celery pydantic-ai; fi
for n in "$@"; do build "$n"; done
