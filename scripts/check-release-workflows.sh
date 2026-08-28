#!/usr/bin/env bash
# Patterns passed to the assertion helpers intentionally remain literal.
# shellcheck disable=SC1003,SC2016
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

command_line_in_job() {
	local file="$1"
	local job="$2"
	local pattern="$3"

	awk -v job="$job" -v pattern="$pattern" '
		$0 ~ "^  " job ":$" { in_job = 1; next }
		in_job && $0 ~ /^  [A-Za-z0-9_-]+:$/ { exit }
		in_job {
			command = $0
			sub(/^[[:space:]]+/, "", command)
			if (command !~ /^#/ && index(command, pattern)) { print NR; exit }
		}
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

	line="$(command_line_in_job "$file" "$job" "$pattern")"
	if [[ -z "$line" ]]; then
		echo "ERROR: $file job '$job' is missing '$pattern'" >&2
		exit 1
	fi
}

assert_job_excludes() {
	local file="$1"
	local job="$2"
	local pattern="$3"
	local line

	line="$(command_line_in_job "$file" "$job" "$pattern")"
	if [[ -n "$line" ]]; then
		echo "ERROR: $file job '$job' still contains forbidden text '$pattern'" >&2
		exit 1
	fi
}

assert_job_runs_on() {
	local file="$1"
	local job="$2"
	local expected="$3"

	ruby -ryaml -e '
		file, job, expected = ARGV
		workflow = YAML.safe_load(File.read(file), aliases: true)
		actual = workflow.fetch("jobs").fetch(job).fetch("runs-on")
		exit if actual == expected

		warn "ERROR: #{file} job #{job.inspect} runs on #{actual.inspect}, expected #{expected.inspect}"
		exit 1
	' "$file" "$job" "$expected"
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

assert_file_excludes_pattern() {
	local file="$1"
	local pattern="$2"

	if grep -Eiq -- "$pattern" "$file"; then
		echo "ERROR: $file matches forbidden pattern '$pattern'" >&2
		exit 1
	fi
}

assert_dispatch_input_required() {
	local file="$1"
	local input="$2"

	if ! awk -v input="$input" '
		$0 == "      " input ":" { in_input = 1; next }
		in_input && $0 ~ /^      [^ ]/ { exit }
		in_input && $0 == "        required: true" { found = 1 }
		END { exit !found }
	' "$file"; then
		echo "ERROR: $file workflow_dispatch input '$input' must be required" >&2
		exit 1
	fi
}

# PR CI must stay runnable without the retired AgentOS ARC pool. Parse the
# workflow instead of matching YAML text so formatting cannot weaken this
# routing contract.
assert_job_runs_on ".github/workflows/ci.yml" "rust" "ubuntu-latest"
assert_job_runs_on ".github/workflows/ci.yml" "sdk" "ubuntu-latest"

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
	"Pack dry-run (manifest sanity + exports completeness)"

# Release-owned coordinates moved with the repository. Keep the image and
# release identities on the current GitHub organization.
for workflow in .github/workflows/*.yml; do
	assert_file_excludes "$workflow" "cnap-tech"
done
assert_file_contains ".github/workflows/release.yml" "ghcr.io/akua-dev/akuapkg:\${tag}"
assert_file_excludes_pattern ".github/workflows/release.yml" 'gh[[:space:]]+release[[:space:]]+upload'
assert_file_excludes ".github/workflows/release.yml" "--clobber"
assert_file_excludes ".github/workflows/release-publish.yml" "gh release download"
assert_file_contains ".github/workflows/release-publish.yml" "source-run-id:"
assert_file_contains ".github/workflows/release.yml" '-f source-run-id="$GITHUB_RUN_ID"'
assert_job_contains ".github/workflows/release.yml" \
	"detect-version" \
	"release-inputs.json"
assert_job_contains ".github/workflows/release-publish.yml" \
	"detect-version" \
	'gh run view "$SOURCE_RUN_ID"'
assert_job_contains ".github/workflows/release-publish.yml" \
	"detect-version" \
	'.status == "in_progress"'
assert_job_contains ".github/workflows/release-publish.yml" \
	"detect-version" \
	'.status == "completed"'
assert_job_contains ".github/workflows/release-publish.yml" \
	"detect-version" \
	'.conclusion == "success"'
assert_job_contains ".github/workflows/release-publish.yml" \
	"detect-version" \
	"release-inputs"
assert_job_contains ".github/workflows/release.yml" \
	"trigger-publish" \
	'-f expected-workflow-commit="$EXPECTED_WORKFLOW_COMMIT" \\'
assert_job_contains ".github/workflows/release-publish.yml" \
	"native-publish" \
	"--pattern 'native-*'"
assert_job_contains ".github/workflows/release-publish.yml" \
	"sdk-publish" \
	'gh run download "$SOURCE_RUN_ID"'
assert_job_contains ".github/workflows/release.yml" \
	"github-release" \
	'if [[ "$GITHUB_EVENT_NAME" == "workflow_dispatch" ]]'

# Homebrew belongs to akua-dev/cli and its dedicated tap lane. Akua core must
# never configure tap credentials, generate a formula, or dispatch a tap writer.
if grep -Eiq 'homebrew|TAP_BUMP|github\.com-tap|Formula/akua\.rb' .github/workflows/release.yml; then
	echo "ERROR: release.yml contains an Akua-core Homebrew/tap side effect or ownership claim" >&2
	exit 1
fi
for file in README.md packages/sdk/README.md docs/sdk-runtime-compat.md; do
	assert_file_excludes_pattern "$file" 'brew[[:space:]]+install[^[:cntrl:]]*akua|homebrew[^[:cntrl:]]*(formula|tap|install|update|publish|release)|(^|[^[:alnum:]_])(formula|tap)([^[:alnum:]_]|$)[^[:cntrl:]]*akua'
done

# Public package-tool docs must follow the executable command surface. Keep
# platform-only and planned verbs out of the standalone Akuapkg reference, and
# keep engine documentation on the package-tool binary name.
docs_contract_failed=0
for verb in attest deploy rollout secret policy audit query infra login logout bench trace cov eval telemetry lint-cli; do
	if grep -Fq -- "akuapkg $verb" docs/cli.md; then
		echo "ERROR: docs/cli.md documents nonexistent standalone verb 'akuapkg $verb'" >&2
		docs_contract_failed=1
	fi
	if [[ -e "site/cli/$verb.html" ]]; then
		echo "ERROR: site/cli/$verb.html publishes nonexistent standalone verb 'akuapkg $verb'" >&2
		docs_contract_failed=1
	fi
done
if ! grep -Fq -- "Akuapkg embeds" docs/embedded-engines.md || \
	grep -Fq -- 'the `akua` binary' docs/embedded-engines.md; then
	echo "ERROR: docs/embedded-engines.md conflates Akuapkg with the platform akua binary" >&2
	docs_contract_failed=1
fi
if grep -Fq -- "## Performance notes" docs/embedded-engines.md || \
	grep -Fq -- "[docs/bench/](bench/)" docs/embedded-engines.md; then
	echo "ERROR: docs/embedded-engines.md contains unsupported or unlinked benchmark claims" >&2
	docs_contract_failed=1
fi
if grep -Fq -- 'akuapkg bench' docs/embedded-engines.md; then
	echo "ERROR: docs/embedded-engines.md recommends nonexistent 'akuapkg bench'" >&2
	docs_contract_failed=1
fi
if (( docs_contract_failed != 0 )); then
	exit 1
fi

# A manual recovery runs workflow code from a green branch, but every source
# checkout must resolve to the requested immutable tag. The workflow verifies
# and propagates that commit; it must never create or move a tag.
for input in expected-source-commit expected-workflow-commit; do
	assert_dispatch_input_required ".github/workflows/release.yml" "$input"
done
assert_job_contains ".github/workflows/release.yml" \
	"detect-version" \
	'EXPECTED_SOURCE_COMMIT: ${{ inputs.expected-source-commit }}'
assert_job_contains ".github/workflows/release.yml" \
	"detect-version" \
	'EXPECTED_WORKFLOW_COMMIT: ${{ inputs.expected-workflow-commit }}'
assert_job_contains ".github/workflows/release.yml" \
	"detect-version" \
	'ACTUAL_WORKFLOW_COMMIT: ${{ github.workflow_sha }}'
assert_job_contains ".github/workflows/release.yml" \
	"detect-version" \
	'if [[ "$source_commit" != "$EXPECTED_SOURCE_COMMIT" ]]'
assert_job_contains ".github/workflows/release.yml" \
	"detect-version" \
	'if [[ "$ACTUAL_WORKFLOW_COMMIT" != "$EXPECTED_WORKFLOW_COMMIT" ]]'
assert_job_contains ".github/workflows/release.yml" \
	"detect-version" \
	'git rev-parse "refs/tags/${tag}^{commit}"'
assert_file_contains ".github/workflows/release.yml" "source_commit: \${{ steps.parse.outputs.source_commit }}"
assert_job_contains ".github/workflows/release.yml" \
	"trigger-publish" \
	"inputs.expected-source-commit"
assert_job_contains ".github/workflows/release.yml" \
	"trigger-publish" \
	"inputs.expected-workflow-commit"
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
if [[ "$(jq -r '.scripts.prepublishOnly' crates/akua-napi/package.json)" != \
	"napi prepublish -t npm --no-gh-release" ]]; then
	echo "ERROR: native meta prepublish must preserve package injection while disabling GitHub Release uploads" >&2
	exit 1
fi
assert_job_excludes ".github/workflows/release-publish.yml" \
	"native-publish" \
	"--ignore-scripts"
assert_job_contains ".github/workflows/release-publish.yml" \
	"sdk-publish" \
	"needs: [detect-version, native-publish]"

# npm requires repository.url to exactly match the trusted GitHub repository.
for manifest in \
	crates/akua-native-engines-npm/package.json \
	crates/akua-napi/package.json \
	crates/akua-napi/npm/*/package.json \
	packages/sdk/package.json; do
	if [[ "$(jq -r '.repository.url' "$manifest")" != "git+https://github.com/akua-dev/akuapkg.git" ]]; then
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
assert_file_excludes ".github/workflows/release.yml" "github.com/akua-dev/akua.git"
assert_file_excludes ".github/workflows/release.yml" "github.com/akua-dev/akua/blob"
assert_file_excludes ".github/workflows/release-publish.yml" "github.com/akua-dev/akua.git"

# The external trusted-publisher identity and the deliberately gated recovery
# command are authoritative release contract, not tribal knowledge.
assert_file_contains "docs/releasing.md" "akua-dev/akuapkg"
assert_file_contains "docs/releasing.md" "release-publish.yml"
assert_file_contains "docs/releasing.md" "Environment: none"
assert_file_contains "docs/releasing.md" "--ref main"
assert_file_contains "docs/releasing.md" "-f expected-source-commit=6452eb662445d2ad7c108128f93b9c55138729bb"
assert_file_contains "docs/releasing.md" '-f expected-workflow-commit="$reviewed_workflow_commit"'
assert_file_contains "docs/releasing.md" 'merged into `main`'
assert_file_contains "docs/releasing.md" 'test "$main_commit" = "$reviewed_workflow_commit"'
assert_file_contains "docs/releasing.md" "CI is green"
assert_file_contains "docs/releasing.md" "all ten packages"
assert_file_contains "docs/releasing.md" "expected source and workflow commit SHAs"
assert_file_contains "docs/releasing.md" "fails before npm publication"
assert_file_contains "docs/releasing.md" "Homebrew is explicitly outside this recovery"
assert_file_contains "docs/releasing.md" "akua-dev/cli"
assert_file_contains "docs/releasing.md" 'dedicated `akua-dev/homebrew-tap` lane'
assert_file_excludes "docs/releasing.md" "updates Homebrew"
assert_file_excludes "docs/releasing.md" "updates the generated Homebrew"
assert_file_contains "Taskfile.yml" "org=akua-dev  repo=akua  workflow=release-publish.yml  environment=none"
assert_file_excludes "Taskfile.yml" "org=cnap-tech  repo=akua  workflow=release.yml"
assert_file_excludes "packages/sdk/README.md" "github.com/cnap-tech/akua"
assert_file_contains "packages/sdk/README.md" "docs/releasing.md"
assert_file_contains "docs/sdk-runtime-compat.md" "release-publish.yml runs npm publish jobs"

echo "Release workflow ordering checks passed."
