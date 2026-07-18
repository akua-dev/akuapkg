# Releasing akua

The build and publish lanes are intentionally separate. `.github/workflows/release.yml`
builds the immutable tag, creates Release assets only for a new tag push, pushes the
container, and dispatches `.github/workflows/release-publish.yml` with the source-run
artifact identity. The publish workflow downloads those Actions artifacts and
publishes npm packages in dependency order. A recovery verifies the existing Release
but never uploads, replaces, or otherwise changes its assets. Neither lane creates or
moves a tag during recovery.

Homebrew is explicitly outside this recovery. Formula ownership belongs to
`akua-dev/cli` and the dedicated `akua-dev/homebrew-tap` lane; Akua core does not
write, dispatch, or claim ownership of that formula.

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

## npm trusted publisher contract

npm trusted-publisher configuration is external registry state. Source code can
request a GitHub OIDC token, but it cannot create or repair the trust relationship.
An npm package administrator must confirm the following identity on all ten packages
before any recovery dispatch:

- GitHub owner/repository: `akua-dev/akua`
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
the exact repository URL `git+https://github.com/akua-dev/akua.git`, as required for
trusted publishing.

## Recovering the partial v0.8.25 release

Recovery is manual and fail-closed. Do not rerun either failed run, retag
`v0.8.25`, recreate its GitHub Release, or upload assets by hand. The existing tag
resolves to commit `6452eb662445d2ad7c108128f93b9c55138729bb`; the manual build lane
checks out that tag, verifies and propagates its commit, and never builds the current
workflow head as the tagged source. Existing Release assets are verified and left
byte-for-byte unchanged, and npm versions already present are skipped before the
missing packages publish.
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
reviewed_workflow_commit=$(gh pr view 69 --repo akua-dev/akua --json mergeCommit --jq '.mergeCommit.oid')
main_commit=$(gh api repos/akua-dev/akua/commits/main --jq '.sha')
test "$main_commit" = "$reviewed_workflow_commit"

gh workflow run release.yml \
  --repo akua-dev/akua \
  --ref main \
  -f tag=v0.8.25 \
  -f expected-source-commit=6452eb662445d2ad7c108128f93b9c55138729bb \
  -f expected-workflow-commit="$reviewed_workflow_commit" \
  -f dry-run=false
```

That single deliberate dispatch rebuilds from the immutable tag, verifies the existing
GitHub Release without changing any asset bytes, publishes `ghcr.io/akua-dev/akua`,
and then dispatches the idempotent npm publish lane from that run's artifacts. It has
no Homebrew side effects. There are no automatic recovery retries.
