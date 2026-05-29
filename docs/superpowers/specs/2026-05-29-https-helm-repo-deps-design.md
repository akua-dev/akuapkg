# HTTPS helm-repo dependencies — design

**Date:** 2026-05-29
**Status:** approved-for-planning
**Scope:** add a fourth dependency source to `akua.toml` — classic HTTPS
Helm chart repositories (`index.yaml` + `.tgz`) — alongside `oci` / `git` /
`path`.

## Problem

akua resolves Helm chart dependencies from three sources: `oci`, `git`, and
`path`. Many widely-used charts are published **only** to classic HTTPS Helm
repositories (an `index.yaml` plus versioned `.tgz` tarballs), never to an OCI
registry — temporal (`https://go.temporal.io/helm-charts`), prometheus,
grafana, and most community charts. Today the only way to consume them is to
vendor the chart tree as a `path` dep and check it into the workspace. That is
manual, drifts from upstream, and bloats the repo.

We want a first-class HTTPS helm-repo source so a chart can be declared,
resolved, pinned, and rendered without vendoring.

## Goals

- Declare an HTTPS helm-repo chart in `akua.toml` with a repo URL, chart name,
  and version (exact **or** semver range).
- Resolve the range against the repo's `index.yaml` at `add` / `lock` time;
  pin the selected version and the `.tgz` content digest in `akua.lock`.
- Render deterministically from the pinned digest — no network, no `index.yaml`
  read at render time.
- Reuse existing machinery: the reqwest+rustls HTTPS transport, the host-keyed
  `host_auth` map for private repos, the content-addressed cache, vendor-first
  resolution, and publish-time vendoring.

## Non-goals (deferred)

- GPG `.prov` provenance verification. Helm provenance is rarely published and
  none of the target charts ship it. Trust is by content digest (see below).
  `.prov` may be added later as opt-in, mirroring how git deps gain nothing
  from upstream signatures today.
- OCI-registry helm charts — already handled by the `oci` source.
- Mutating an existing chart's `values.schema.json` or templates.

## Trust model — content-pin, like `git`

akua already consumes unsigned upstreams: `git` and `path` deps are **not**
cosign-verified. They are pinned by content (commit SHA / tree SHA256) in
`akua.lock`, and re-fetch verifies that pin. The "signed + attested by default"
invariant governs akua's **own `publish` output**, not third-party upstreams.

HTTPS helm-repo charts adopt the identical pattern:

- `akua add` / `akua lock` resolve the version, download the `.tgz`, compute
  its tree SHA256, and write `digest = "sha256:<hex>"` into `akua.lock`.
- Every subsequent fetch verifies the downloaded tarball's tree hash against
  the pinned digest; a mismatch fails the resolve hard (same as OCI's
  digest-mismatch path).
- No signature is required. `.prov`/GPG remains a future opt-in.

## Authoring shape

A fourth source discriminant, using Helm's own vocabulary:

```toml
[dependencies.temporal]
repo    = "https://go.temporal.io/helm-charts"   # source discriminant
chart   = "temporal"
version = "0.62.0"                                # exact, or a range like ">=0.60, <0.63"
```

- `repo` is the source field (exclusive with `oci` / `git` / `path`).
- `chart` is required for `repo` deps (the entry name within `index.yaml`).
- `version` is required and may be an exact version or a semver range.
- `replace = { path = "..." }` works as for `oci`/`git`: the canonical
  repo/chart/version stays recorded for audit, files come from the local fork.

## Protocol — the Helm Repository API

A new `helm_repo_fetcher` module (modeled on `git_fetcher`):

1. `GET {repo}/index.yaml`. This is the standard Helm Repository index: a YAML
   document mapping `entries.<chart>[]` to chart metadata, each with a
   `version` and a `urls[]` list of tarball locations. URLs may be absolute or
   relative to the repo base — resolve relative URLs against `{repo}/`.
2. **Version selection.** Collect all `entries.<chart>[].version` values.
   - Exact version → require an exact match.
   - Range → parse with the `semver` crate, select the highest version
     satisfying the range. Pre-release versions are excluded unless the range
     names one explicitly (cargo/helm convention).
   - No match → structured error listing the available versions.
3. Resolve the selected entry's first `urls[]` entry to an absolute `.tgz` URL.
4. Download the `.tgz` over the existing reqwest+rustls blocking transport.
5. Extract into the content-addressed cache at `~/.cache/akua/helm`
   (`$XDG_CACHE_HOME/akua/helm`), compute the tree SHA256 (the same
   `resolve_path` hashing OCI uses post-extract).
6. **Auth:** look up credentials in the host-keyed `host_auth` map (the
   `--auth <prefix>=<user>:<pass>` surface already used by `vendor`/git).
   Anonymous when no match. The `index.yaml` GET and the `.tgz` GET both use it.
7. **Offline:** when `opts.offline`, skip the network; require a lockfile-pinned
   digest and a populated cache entry, exactly like `oci_fetcher::fetch_from_cache`.

## Determinism

- `index.yaml` is consulted **only** at `akua add` / `akua lock`. It writes the
  resolved exact version and the `.tgz` digest into `akua.lock`.
- `akua render` resolves from the pinned digest + cache; it never reads
  `index.yaml` and never hits the network. Same inputs + same lockfile + same
  akua version → byte-identical output, satisfying the determinism invariant.

## Data-model changes

`crates/akua-core/src/mod_file.rs`:

- `Dependency`: add `repo: Option<String>` and `chart: Option<String>`.
- `DependencySource`: add `Helm`.
- `DependencySpec`: add `Helm { repo: &str, chart: &str, version: &str }`.
- `source()`: `repo` set (and only `repo`) → `DependencySource::Helm`.
- `validate()`: a `repo` dep requires `chart` and `version`; `repo` must be an
  `https://` (or `http://` for in-cluster mirrors) URL with no embedded
  userinfo (reuse `host_auth::url_has_userinfo`); must not set `tag`/`rev`.
- New `ManifestError` variants → new `E_MANIFEST_*` codes:
  `HelmMissingChart`, `HelmMissingVersion`, `HelmUrlHasUserInfo`.

## Resolver changes

`crates/akua-core/src/chart_resolver.rs`:

- `ResolvedSource`: add `Helm { repo, chart, version, digest }`.
- `to_locked_fields()`: project to the locked triple (source string, digest,
  replace) — digest is `sha256:<hex>`.
- `resolve_with_options`: add a `DependencySpec::Helm` arm calling a new
  `resolve_helm` (mirrors `resolve_oci`: cache_root, expected digest, offline
  branch, online fetch via `helm_repo_fetcher`).
- `VendorKind`: add `Helm { repo, chart, version }` so vendor-first and
  publish-time vendoring work unchanged.

## Lockfile changes

`crates/akua-core/src/lock_file.rs`: a `repo` dep locks as

```toml
[[package]]
name    = "temporal"
source  = "https://go.temporal.io/helm-charts"   # repo URL
chart   = "temporal"
version = "0.62.0"                                # resolved exact version
digest  = "sha256:<hex>"
```

Re-pull verifies `digest`. `replace` provenance recorded as for oci/git.

## CLI / SDK surface (one contract)

- `akua add` gains a helm-repo form:
  `akua add temporal --repo https://go.temporal.io/helm-charts --chart temporal --version ">=0.60,<0.63"`.
  Resolves the range, writes `akua.toml` + `akua.lock`.
- The napi shim + `packages/sdk` `Akua.add()` route the new fields through.
- `docs/lockfile-format.md`, `docs/package-format.md`, and the dependency-source
  table in `docs/cli.md` document the `repo`/`chart` fields.

## Security

- `repo` URLs with embedded `user:pass@` rejected at parse (as for git).
- Tarball extraction goes through the existing path-escape guard; a malicious
  `.tgz` with `../` members cannot escape the cache dir.
- `akua publish` strips `replace` for helm deps too (existing behavior is
  source-agnostic).
- Credentials only ever come from the `host_auth` map; akua never reads
  `~/.netrc`, `helm`'s `repositories.yaml`, or ambient credential stores.

## Error handling

New `ChartResolveError` variants wrapping `HelmRepoFetchError`
(index fetch failure, chart-not-found, no-version-satisfies-range with the
available list, tarball download failure, digest mismatch, offline-without-cache).
Each maps to a stable `E_*` code so agents branch precisely.

## Testing strategy

- `mod_file` unit tests: `repo` source discrimination; validation (missing
  chart, missing version, userinfo rejection, mutual exclusion with oci/git/path).
- `helm_repo_fetcher` unit tests against a local fixture `index.yaml` + `.tgz`
  served from a temp dir / `file://`-style indirection (no live network, per the
  git_fetcher test pattern), covering: exact-version select, range select
  (highest match), relative vs absolute tarball URLs, chart-not-found,
  no-satisfying-version, digest pin + mismatch, offline-from-cache.
- `chart_resolver` test: a `repo` dep resolves to a chart dir + `ResolvedSource::Helm`.
- Lockfile round-trip test for the `Helm` source.
- Integration: a golden Package depending on a fixture helm repo renders
  deterministically.

## Open questions

None blocking. `.prov`/GPG and OCI-fallback mirrors are explicit non-goals.
