#!/usr/bin/env bash
# Patterns passed to the assertion helpers intentionally remain literal.
# shellcheck disable=SC2016
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

assert_file_contains() {
	local file="$1"
	local pattern="$2"

	if ! grep -Fq -- "$pattern" "$file"; then
		echo "ERROR: $file is missing '$pattern'" >&2
		exit 1
	fi
}

assert_file_excludes() {
	local file="$1"
	local pattern="$2"

	if grep -Fq -- "$pattern" "$file"; then
		echo "ERROR: $file still contains forbidden text '$pattern'" >&2
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

# Release-owned coordinates moved with the repository. Keep the image, release,
# and generated Homebrew formula identities on the current GitHub organization.
for workflow in .github/workflows/*.yml; do
	assert_file_excludes "$workflow" "cnap-tech"
done
assert_file_contains ".github/workflows/release.yml" "ghcr.io/akua-dev/akua:\${tag}"
assert_file_contains ".github/workflows/release.yml" "git@github.com-tap:akua-dev/homebrew-tap.git"
assert_file_contains ".github/workflows/release.yml" "homepage \"https://github.com/akua-dev/akua\""
assert_file_contains ".github/workflows/release.yml" "gh release upload \"\${TAG}\" dist/*.tar.gz dist/*.zip dist/*.sha256 --clobber"

# A manual recovery runs workflow code from a green branch, but every source
# checkout must resolve to the requested immutable tag. The workflow verifies
# and propagates that commit; it must never create or move a tag.
assert_job_contains ".github/workflows/release.yml" \
	"detect-version" \
	'git rev-parse "refs/tags/${tag}^{commit}"'
assert_file_contains ".github/workflows/release.yml" "source_commit: \${{ steps.parse.outputs.source_commit }}"
assert_file_contains ".github/workflows/release.yml" "EXPECTED_SOURCE_COMMIT: \${{ needs.detect-version.outputs.source_commit }}"
assert_file_contains ".github/workflows/release.yml" "EXPECTED_WORKFLOW_COMMIT: \${{ github.workflow_sha }}"
assert_file_contains ".github/workflows/release.yml" 'WORKFLOW_REF: ${{ github.ref_name }}'
assert_job_contains ".github/workflows/release.yml" \
	"trigger-publish" \
	'--ref "$WORKFLOW_REF"'
assert_job_contains ".github/workflows/release.yml" \
	"trigger-publish" \
	'-f expected-source-commit="$EXPECTED_SOURCE_COMMIT"'
assert_job_contains ".github/workflows/release.yml" \
	"trigger-publish" \
	'-f expected-workflow-commit="$EXPECTED_WORKFLOW_COMMIT"'
for input in expected-source-commit expected-workflow-commit; do
	assert_file_contains ".github/workflows/release-publish.yml" "$input:"
done
assert_job_contains ".github/workflows/release-publish.yml" \
	"detect-version" \
	'ACTUAL_WORKFLOW_COMMIT: ${{ github.sha }}'
assert_job_contains ".github/workflows/release-publish.yml" \
	"detect-version" \
	'if [[ "$ACTUAL_WORKFLOW_COMMIT" != "$EXPECTED_WORKFLOW_COMMIT" ]]'
assert_job_contains ".github/workflows/release-publish.yml" \
	"detect-version" \
	'if [[ "$source_commit" != "$EXPECTED_SOURCE_COMMIT" ]]'
for job in wasm-bundle native-build sdk-build cli-build github-release docker; do
	assert_job_contains ".github/workflows/release.yml" \
		"$job" \
		'ref: ${{ needs.detect-version.outputs.source_commit }}'
done
assert_job_contains ".github/workflows/release-publish.yml" \
	"native-publish" \
	'ref: ${{ needs.detect-version.outputs.source_commit }}'
for workflow in .github/workflows/release.yml .github/workflows/release-publish.yml; do
	if grep -Eq '^[[:space:]]*(git tag|git push .*refs/tags/|gh release delete)' "$workflow"; then
		echo "ERROR: $workflow contains a tag/Release replacement command" >&2
		exit 1
	fi
done

# Recovery publication is tokenless and idempotent. Every npm package is
# published by release-publish.yml, in dependency order, through the same
# probe-before-publish helper. The SDK must not be a one-off duplicate publish.
assert_file_contains ".github/workflows/release-publish.yml" "id-token: write"
if grep -Eq '^[[:space:]]*NODE_AUTH_TOKEN:' .github/workflows/release-publish.yml; then
	echo "ERROR: release-publish.yml must not configure NODE_AUTH_TOKEN" >&2
	exit 1
fi
assert_job_contains ".github/workflows/release-publish.yml" \
	"sdk-publish" \
	'npm view "${name}@${version}" version'
assert_job_contains ".github/workflows/release-publish.yml" \
	"sdk-publish" \
	"publish_one sdk --ignore-scripts"
assert_before ".github/workflows/release-publish.yml" \
	"native-publish" \
	"publish_one crates/akua-native-engines-npm" \
	"publish_one crates/akua-napi"
assert_job_contains ".github/workflows/release-publish.yml" \
	"sdk-publish" \
	"needs: [detect-version, native-publish]"

# npm requires repository.url to exactly match the trusted GitHub repository.
for manifest in \
	crates/akua-native-engines-npm/package.json \
	crates/akua-napi/package.json \
	crates/akua-napi/npm/*/package.json \
	packages/sdk/package.json; do
	if [[ "$(jq -r '.repository.url' "$manifest")" != "git+https://github.com/akua-dev/akua.git" ]]; then
		echo "ERROR: $manifest has stale npm repository provenance" >&2
		exit 1
	fi
done
if (( $(grep -Fc '.repository.url = $repo' .github/workflows/release-publish.yml) < 2 )); then
	echo "ERROR: release-publish.yml must normalize native + engine provenance after tag checkout" >&2
	exit 1
fi
assert_job_contains ".github/workflows/release.yml" \
	"sdk-build" \
	'.repository.url = $repo'
assert_job_contains ".github/workflows/release.yml" \
	"sdk-build" \
	'.homepage = $homepage'

# The external trusted-publisher identity and the deliberately gated recovery
# command are authoritative release contract, not tribal knowledge.
assert_file_contains "docs/releasing.md" "akua-dev/akua"
assert_file_contains "docs/releasing.md" "release-publish.yml"
assert_file_contains "docs/releasing.md" "Environment: none"
assert_file_contains "docs/releasing.md" "--ref main -f tag=v0.8.25"
assert_file_contains "docs/releasing.md" "only after the source PR CI is green"
assert_file_contains "docs/releasing.md" "all ten packages"
assert_file_contains "docs/releasing.md" "expected source and workflow commit SHAs"
assert_file_contains "docs/releasing.md" "fails before npm publication"
assert_file_contains "Taskfile.yml" "org=akua-dev  repo=akua  workflow=release-publish.yml  environment=none"
assert_file_excludes "Taskfile.yml" "org=cnap-tech  repo=akua  workflow=release.yml"
assert_file_excludes "packages/sdk/README.md" "github.com/cnap-tech/akua"
assert_file_contains "packages/sdk/README.md" "docs/releasing.md"
assert_file_contains "docs/sdk-runtime-compat.md" "release-publish.yml runs npm publish jobs"

echo "Release workflow ordering checks passed."
