# Releasing Akuapkg

Akuapkg publishes the standalone `akuapkg` binary from `akua-dev/akuapkg`. The Akua platform CLI is a separate artifact named `akua`; do not publish or document a platform CLI release from this repository.

The canonical distribution coordinates are:

- Repository: `https://github.com/akua-dev/akuapkg`
- GitHub releases: `https://github.com/akua-dev/akuapkg/releases`
- Homebrew formula: `akua-dev/tap/akuapkg`
- Container: `ghcr.io/akua-dev/akuapkg`

As of August 14, 2026, `v0.9.3` is a source tag without a GitHub Release. Do not describe that version as published or installable from release assets until the release workflow succeeds.

The build and publish lanes are intentionally separate. `.github/workflows/release.yml`
builds the immutable tag, creates Release assets for a new tag, pushes the
container, and dispatches `.github/workflows/release-publish.yml` with the source-run
artifact identity. The publish workflow downloads those Actions artifacts and
publishes npm packages in dependency order. A recovery verifies the existing Release
and never replaces or otherwise changes its assets. If the original run failed before
creating a Release, the recovery lane can create it only with the explicit
`create-missing-release` opt-in. Neither lane creates or moves a tag during recovery.

Homebrew is explicitly outside this recovery. Formula ownership belongs to `akua-dev/cli` and the dedicated `akua-dev/homebrew-tap` lane; Akuapkg does not write, dispatch, or claim ownership of that formula.

## Normal releases

Land and verify release changes on `main`, batch them, and push one immutable
`v<semver>` tag only when a human explicitly requests the release. The tag triggers
the build workflow, which derives the workspace and package version from the tag;
the committed `Cargo.toml` version remains a development placeholder. Stable tags
publish npm packages under `latest` and update the container's `latest` tag.
Prerelease tags publish npm packages under `next`, mark the GitHub Release as a
prerelease, and do not update the container's `latest` tag.

Do not delete and re-push a release tag. If a tagged run fails, use the SHA-bound
recovery path below after correcting and reviewing the workflow on `main`.

## Recovering a failure before GitHub Release creation

If a tag run fails before `github-release`, first correct and review the workflow on
`main`. Do not move the tag, upload assets by hand, or rerun the outdated tagged
workflow. Dispatch the corrected workflow from its exact reviewed commit. Bind the
run to both the immutable tag commit and the reviewed workflow commit, and explicitly
authorize creation of the still-missing Release:

```sh
reviewed_workflow_commit=$(gh api repos/akua-dev/akuapkg/commits/main --jq '.sha')

gh workflow run release.yml \
  --repo akua-dev/akuapkg \
  --ref main \
  -f tag=v0.9.5 \
  -f expected-source-commit=be62c125e9816aa1a440de9afecb4b6fc9a8d487 \
  -f expected-workflow-commit="$reviewed_workflow_commit" \
  -f create-missing-release=true \
  -f dry-run=false
```

The dispatch rebuilds every artifact from the tag. It creates the Release only if it
is still absent; if a Release already exists, it verifies it and never uploads or
replaces assets. Keep `create-missing-release` false for ordinary recovery of an
existing immutable Release.

The `@akua-dev/native` meta-package must publish with lifecycle scripts enabled. Its
`prepublishOnly` hook runs `napi prepublish -t npm --no-gh-release`: `napi prepublish`
injects the generated per-platform `optionalDependencies` and copies the native
addons into the package, while `--no-gh-release` prevents the hook from uploading
assets to the immutable GitHub Release. Do not add `--ignore-scripts` to the native
publish; that flag is only valid for the separately staged SDK publish.

## npm trusted publisher contract

npm trusted-publisher configuration is external registry state. Source code can
request a GitHub OIDC token, but it cannot create or repair the trust relationship.
An npm package administrator must confirm the following identity on all ten packages
before any recovery dispatch:

- GitHub owner/repository: `akua-dev/akuapkg`
- Workflow filename: `release-publish.yml`
- Environment: none (leave the optional npm Environment field empty)
- Allowed action: `npm publish`

The affected packages are:

- `@akua-dev/native-engines`
- `@akua-dev/native-darwin-arm64`
- `@akua-dev/native-darwin-x64`
- `@akua-dev/native-linux-arm64-gnu`
- `@akua-dev/native-linux-arm64-musl`
- `@akua-dev/native-linux-x64-gnu`
- `@akua-dev/native-linux-x64-musl`
- `@akua-dev/native-win32-x64-msvc`
- `@akua-dev/native`
- `@akua-dev/sdk`

Change each package under npm package settings → Trusted Publisher. Do not create,
request, or pass an npm token: the workflow has `id-token: write`, uses GitHub-hosted
runners, and deliberately has no `NODE_AUTH_TOKEN`. Each package manifest also uses
the exact repository URL `git+https://github.com/akua-dev/akuapkg.git`, as required for
trusted publishing.

## Recovering the partial v0.8.25 release

Recovery is manual and fail-closed. Do not rerun either failed run, retag
`v0.8.25`, recreate its GitHub Release, or upload assets by hand. The existing tag
resolves to commit `6452eb662445d2ad7c108128f93b9c55138729bb`; the manual build lane
checks out that tag, verifies and propagates its commit, and never builds the current
workflow head as the tagged source. The existing Release is verified; all 19 assets
are left byte-for-byte unchanged, and npm versions already present are skipped before
the missing packages publish.
Recovery starts a new full source build so the publisher can consume that run's
short-lived verified Actions artifacts; do not dispatch `release-publish.yml` alone
with a stale or guessed run ID.
The build lane passes its expected source and workflow commit SHAs to the publish
lane. The publisher verifies that the tag still resolves to the expected source and
that `github.sha` is the reviewed workflow commit; if either ref advances, recovery
fails before npm publication.

A captain may run this recovery only after PR #69 CI is green, the corrected
commit is merged into `main`, that exact reviewed merge commit is verified as the
current `main` commit, and an npm administrator has confirmed the
trusted-publisher identity above for all ten packages:

```sh
reviewed_workflow_commit=$(gh pr view 69 --repo akua-dev/akuapkg --json mergeCommit --jq '.mergeCommit.oid')
main_commit=$(gh api repos/akua-dev/akuapkg/commits/main --jq '.sha')
test "$main_commit" = "$reviewed_workflow_commit"

gh workflow run release.yml \
  --repo akua-dev/akuapkg \
  --ref main \
  -f tag=v0.8.25 \
  -f expected-source-commit=6452eb662445d2ad7c108128f93b9c55138729bb \
  -f expected-workflow-commit="$reviewed_workflow_commit" \
  -f dry-run=false
```

That single deliberate dispatch rebuilds from the immutable tag, verifies the existing
GitHub Release without changing any asset bytes, publishes `ghcr.io/akua-dev/akuapkg`,
and then dispatches the idempotent npm publish lane from that run's artifacts. It has
no Homebrew side effects. There are no automatic recovery retries.
