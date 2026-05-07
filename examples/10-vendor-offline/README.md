# Example 10 — vendor-offline (render without network or auth)

> **Renders end-to-end** off the local vendor tree. No registry, no
> network, no credentials needed at render time. The same Package
> renders identically inside an air-gapped environment or behind a
> firewall once `akua vendor add` has staged the bytes.

A Package whose dep is materialized into `.akua/vendor/<name>/` so
render works without re-fetching from the canonical source.

## Why vendor

The resolver already prefers `.akua/vendor/<name>/` when it exists
(see `chart_resolver::resolve_from_vendor`). `akua vendor add` is the
public CLI verb that populates that path from a declared dep. The
contract:

- The dep declaration in `akua.toml` stays canonical (`path` / `oci`
  / `git`). It records *what* the dep is.
- `.akua/vendor/<name>/` records *the bytes you want render to use.*
  Always preferred when present.
- `akua.lock` pins the vendored-tree digest so a re-checkout of the
  workspace produces a byte-identical render.

This is the same story Go's `vendor/` directory tells: "commit the
deps so your build is reproducible without a network round-trip."

## What's here

| file | purpose |
|---|---|
| `package.k` | KCL Package; renders the chart at `.akua/vendor/upstream`. |
| `akua.toml` | Manifest — declares `upstream` as a path dep. |
| `inputs.example.yaml` | Auto-discovered when `--inputs` is omitted. |
| `upstream-chart/` | The "canonical" chart source. In a production install pipeline this would be an OCI ref or a private git repo. |
| `.akua/vendor/upstream/` | Created by `akua vendor add` (NOT checked in by default in this example, so you see the verb populate it). |

## Try it

```sh
# 1. From a fresh checkout, no vendor tree exists yet:
ls .akua/vendor/  # → no such file or directory

# 2. Materialize the vendor tree from the declared source:
akua vendor add upstream
# → vendor upstream
#     source  path ./upstream-chart
#     path    .akua/vendor/upstream
#     digest  sha256:...
#     wrote   true

# 3. Confirm the resolver prefers the vendored copy. Even if the
#    original `./upstream-chart/` were deleted, render would still
#    succeed because the resolver finds .akua/vendor/upstream first:
akua render --out ./rendered

# 4. Verify integrity later — `akua vendor check` re-hashes the
#    vendor tree and compares against akua.lock:
akua vendor check
# → ok

# 5. List what's vendored, including any orphan trees that no longer
#    correspond to a dep in akua.toml:
akua vendor list
```

## When vendoring matters

For interactive Package authoring, vendoring is overkill — let the
resolver fetch from OCI or git on every render.

It earns its keep when:

- **Air-gapped render.** The render host has no path to the source
  registry. Vendor at bootstrap from a host that does; commit; render
  reproducibly later.
- **Render on a credential-free host.** Private OCI / git deps need
  auth. If render runs in a least-privilege context that shouldn't
  hold those credentials, vendor at a one-shot privileged step
  upstream.
- **Reproducibility under registry churn.** OCI registries and git
  hosts can disappear, garbage-collect old tags, or rewrite history.
  Vendoring pins the bytes locally; the digest in `akua.lock` keeps
  the post-checkout render deterministic.
- **Per-customer install repos** (the cnap install-as-Package use
  case). The install pipeline mints a per-install token, vendors the
  composed Package's bytes once at bootstrap, and commits the result.
  Subsequent renders need neither the token nor network access.

## Out of scope (for now)

- **Recursive transitive vendoring.** `akua vendor add upstream`
  vendors `upstream` only. If `upstream` itself depends on a chart
  that needs network at render time, vendor that too — track CI
  drift with `akua vendor check`.
- **Workspace-wide `vendor add` (no name).** Currently `add` takes
  exactly one dep name. Looping is the caller's job.
- **Vendor-from-`oci`/`git`.** This example uses a path dep so it
  has zero runtime requirements. The same `vendor add` flow works
  for OCI and git deps once you have the appropriate credentials
  configured for the one-shot fetch.

## Path-escape safety

`akua vendor add` rejects:

- Absolute paths in `path = "..."`. `path = "/etc"` → `E_PATH_ESCAPE`.
- Relative paths that canonicalize outside the workspace. `path =
  "../sibling"` → `E_PATH_ESCAPE`.

Same workspace-local invariant the resolver enforces for path deps —
vendor cannot be used as a side-channel to copy arbitrary host bytes
into an install repo.
