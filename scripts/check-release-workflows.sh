#!/usr/bin/env bash
set -euo pipefail

line_in_job() {
	local file="$1"
	local job="$2"
	local pattern="$3"

	awk -v job="$job" -v pattern="$pattern" '
		$0 ~ "^  " job ":$" { in_job = 1; next }
		in_job && $0 ~ /^  [A-Za-z0-9_-]+:$/ { exit }
		in_job && index($0, pattern) { print NR; exit }
	' "$file"
}

assert_before() {
	local file="$1"
	local job="$2"
	local first="$3"
	local second="$4"
	local first_line second_line

	first_line="$(line_in_job "$file" "$job" "$first")"
	second_line="$(line_in_job "$file" "$job" "$second")"

	if [[ -z "$first_line" ]]; then
		echo "ERROR: $file job '$job' is missing step '$first'" >&2
		exit 1
	fi
	if [[ -z "$second_line" ]]; then
		echo "ERROR: $file job '$job' is missing step '$second'" >&2
		exit 1
	fi
	if (( first_line >= second_line )); then
		echo "ERROR: $file job '$job' must run '$first' before '$second'" >&2
		echo "       '$first' line $first_line, '$second' line $second_line" >&2
		exit 1
	fi
}

assert_job_contains() {
	local file="$1"
	local job="$2"
	local pattern="$3"
	local line

	line="$(line_in_job "$file" "$job" "$pattern")"
	if [[ -z "$line" ]]; then
		echo "ERROR: $file job '$job' is missing '$pattern'" >&2
		exit 1
	fi
}

# Release builds must install from the committed lockfile before mutating
# package manifests to the tag version. Mutating first invalidates
# --frozen-lockfile and can also force unpublished tag versions to resolve.
assert_before ".github/workflows/release.yml" \
	"native-build" \
	"Install napi deps" \
	"Bump package.json version from the tag"
assert_before ".github/workflows/release.yml" \
	"native-build" \
	"Bump package.json version from the tag" \
	"Build native (release)"
assert_before ".github/workflows/release-publish.yml" \
	"native-publish" \
	"Install napi deps" \
	"Bump package.json version from the tag"
assert_before ".github/workflows/release-publish.yml" \
	"native-publish" \
	"Bump package.json version from the tag" \
	"Generate per-platform npm dirs"
assert_job_contains ".github/workflows/release.yml" \
	"native-build" \
	'.dependencies["@akua-dev/native-engines"] = $v'
assert_job_contains ".github/workflows/release-publish.yml" \
	"native-publish" \
	'.dependencies["@akua-dev/native-engines"] = $v'

# The SDK bundle build does not need the just-tagged native package; only the
# staged npm manifest does. Build first, then pin the manifest before pack/upload.
assert_before ".github/workflows/release.yml" \
	"sdk-build" \
	"Build SDK (bun bundle + tsc declarations)" \
	"Bump package.json version + native dep pin from the tag"
assert_before ".github/workflows/release.yml" \
	"sdk-build" \
	"Bump package.json version + native dep pin from the tag" \
	"Pack dry-run (manifest sanity)"

echo "Release workflow ordering checks passed."
