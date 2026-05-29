#!/usr/bin/env bash
# Rewrite the [workspace.package] version in the root Cargo.toml.
#
# Releases derive the binary version from the pushed git tag, not from a
# hand-edited Cargo.toml — see release.yml. CARGO_PKG_VERSION is baked into
# SLSA provenance, OCI annotations, the HTTP user-agent, `akua -V`, and the
# `version`/`whoami` verbs, so the value the binary reports must equal the
# tag it ships under. This script is the single mutation point CI calls
# before building; keeping it out of the workflow YAML lets the same command
# run locally to reproduce a tagged build.
#
# Portable across GNU and BSD sed (macOS runners): no `sed -i`, write+mv
# instead. Only the column-0 `version = "..."` line matches — dependency
# versions are inline (`serde = { version = "1" }`) and members inherit via
# `version.workspace = true`, so there is exactly one such line.
set -euo pipefail

version="${1:?usage: set-cargo-version.sh <version>}"

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
manifest="$root/Cargo.toml"

tmp="$(mktemp)"
sed 's/^version = ".*"$/version = "'"$version"'"/' "$manifest" >"$tmp"
mv "$tmp" "$manifest"

result="$(grep -m1 '^version = ' "$manifest")"
if [ "$result" != "version = \"$version\"" ]; then
	echo "set-cargo-version: expected 'version = \"$version\"', got '$result'" >&2
	exit 1
fi
echo "Cargo.toml workspace version -> $version"
