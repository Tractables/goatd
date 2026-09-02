#!/usr/bin/env bash
# The core crate, every binding beside it and the citation metadata carry one
# version, and a release tag names that same version. Nothing else enforces it:
# a tag whose version had not been bumped would build a wheel labelled with the
# previous release and publish it to PyPI, where a version can be yanked but
# never replaced, and would leave the citation naming the version before it.
#
# CI runs this with no argument to check the crates agree. The release
# workflows pass the tag they are building.
set -euo pipefail
shopt -s nullglob

cd "$(dirname "$0")/../.."

version_of() {
  cargo metadata --no-deps --format-version 1 --manifest-path "$1" |
    jq -r '.packages[0].version'
}

core=$(version_of Cargo.toml)
status=0

for manifest in bindings/*/Cargo.toml; do
  found=$(version_of "$manifest")
  if [ "$found" != "$core" ]; then
    echo "$manifest is version $found, but the core crate is $core"
    status=1
  fi
done

# `CITATION.cff` is what GitHub's citation button reads; the BibTeX entry in
# `README.md` is what people paste into a paper. A stale version here is
# copied by hand and is not yanked afterwards.
cff=$(sed -n 's/^version: *//p' CITATION.cff)
if [ "$cff" != "$core" ]; then
  echo "CITATION.cff is version ${cff:-missing}, but the core crate is $core"
  status=1
fi

readme=$(sed -n 's/^ *note *= *{.*version \([^}]*\)}.*/\1/p' README.md)
if [ "$readme" != "$core" ]; then
  echo "the README citation is version ${readme:-missing}, but the core crate is $core"
  status=1
fi

if [ "$#" -gt 0 ] && [ -n "$1" ]; then
  if [ "${1#v}" != "$core" ]; then
    echo "tag $1 does not name version $core"
    status=1
  fi
fi

if [ "$status" -eq 0 ]; then
  echo "every crate and the citation name version $core"
fi
exit "$status"
