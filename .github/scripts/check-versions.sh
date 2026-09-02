#!/usr/bin/env bash
# The core crate and every binding beside it carry one version, and a release
# tag names that same version. Nothing else enforces it: a tag whose version
# had not been bumped would build a wheel labelled with the previous release
# and publish it to PyPI, where a version can be yanked but never replaced.
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

if [ "$#" -gt 0 ] && [ -n "$1" ]; then
  if [ "${1#v}" != "$core" ]; then
    echo "tag $1 does not name version $core"
    status=1
  fi
fi

if [ "$status" -eq 0 ]; then
  echo "every crate is version $core"
fi
exit "$status"
