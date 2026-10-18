#!/usr/bin/env bash
# Seeds a fuzz corpus directory (nightly `fuzz` and `fuzz-corpus` jobs), from the repository root:
#
#   fuzz/seeds.sh <target> <corpus dir> [--fetched]
#
# Seeds are this repository's own files. `--fetched` adds the corpus sources `corpus/fetch.sh`
# downloads (xschem_read: real PDK and xschem library files); those include GPL xschem data, so
# a corpus seeded with them is never committed (the `fuzz-corpus` job omits it).
set -eu # no pipefail: `head` closes the find pipe early on purpose

target="$1"
dir="$2"
fetched="${3:-}"
mkdir -p "$dir"
case "$target" in
  xschem_read)
    roots="determinism/xschem"
    if [ "$fetched" = --fetched ]; then
      corpus/fetch.sh >/dev/null
      roots="$roots corpus/.cache/src"
    fi
    # shellcheck disable=SC2086 # roots is a list of paths without spaces
    find $roots \( -name '*.sch' -o -name '*.sym' \) -type f -size -64k | head -n 4000 |
      while read -r f; do cp "$f" "$dir/$(echo "$f" | sha1sum | cut -c1-16)"; done
    ;;
  sfyx_import) ;; # libFuzzer starts from the checked-in corpus
  *)
    echo "unknown fuzz target: $target" >&2
    exit 2
    ;;
esac
echo "$(find "$dir" -type f | wc -l) corpus inputs in $dir"
