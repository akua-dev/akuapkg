# example 12 — vendor-offline (render without network or auth)

> **Renders end-to-end** off the local vendor tree. No registry, no
> network, no credentials needed at render time. The same Package
> renders identically inside an air-gapped environment or behind a
> firewall once `akuapkg vendor add` has staged the bytes.

A Package whose dep is materialized into `.akua/vendor/<name>/` so
render works without re-fetching from the canonical source. This
example takes the offline guarantee literally — the canonical
`upstream-chart/` source **is not present in the checkout**. Render
succeeds because the resolver finds the bytes under
`.akua/vendor/upstream/`.

## Why vendor

The resolver prefers `.akua/vendor/<name>/` when it exists for *every*
dep kind — path, OCI, and git alike (see
`chart_resolver::resolve_with_options`). `akuapkg vendor add` is the
public CLI verb that populates that path from a declared dep and pins
the digest in `akua.lock`. The contract:

- The dep declaration in `akua.toml` stays canonical (`path` / `oci`
  / `git`). It records *what* the dep is.
- `.akua/vendor/<name>/` records *the bytes you want render to use.*
  Always preferred when present — across all dep kinds.
- `akua.lock` pins the vendored-tree digest so a re-checkout of the
  workspace produces a byte-identical render.

This is the same story Go's `vendor/` directory tells: "commit the
deps so your build is reproducible without a network round-trip."

## What's here

| file | purpose |
|---|---|
| `package.k` | KCL Package; renders the chart at `.akua/vendor/upstream`. |
| `akua.toml` | Manifest — declares `upstream` as a path dep pointing at `./upstream-chart`. |
| `akua.lock` | Pins the vendor tree's digest. Committed. |
| `inputs.example.yaml` | Auto-discovered when `--inputs` is omitted. |
| `.akua/vendor/upstream/` | The vendored chart bytes. Committed — this is the offline-render guarantee. |
| `rendered/` | Golden-rendered output the integration test diffs against. |
| ~~`upstream-chart/`~~ | **Intentionally absent.** In a production install pipeline this would be the OCI ref / private git repo / inaccessible-at-render-time canonical source. The point of this example is that render works without it. |

## Try it

```sh
# 1. Confirm the canonical source is gone:
ls upstream-chart/  # → No such file or directory

# 2. Confirm the vendor tree is committed:
ls .akua/vendor/upstream/  # → Chart.yaml  templates/

# 3. Render — succeeds without network, auth, or canonical source:
akuapkg render --out ./rendered

# 4. Verify integrity — `akuapkg vendor check` re-hashes the vendor tree
#    and compares against akua.lock:
akuapkg vendor check
# → ok

# 5. List what's vendored, including any orphan trees that no longer
#    correspond to a dep in akua.toml:
akuapkg vendor list
```

To regenerate the vendor tree from a canonical source (e.g., during
development before committing), restore `upstream-chart/` and run
`akuapkg vendor add upstream`.

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

- **Recursive transitive vendoring.** `akuapkg vendor add upstream`
  vendors `upstream` only. If `upstream` itself depends on a chart
  that needs network at render time, vendor that too — track CI
  drift with `akuapkg vendor check`.
- **Workspace-wide `vendor add` (no name).** Currently `add` takes
  exactly one dep name. Looping is the caller's job.

## Path-escape safety

`akuapkg vendor add` rejects:

- Absolute paths in `path = "..."`. `path = "/etc"` → `E_PATH_ESCAPE`.
- Relative paths that canonicalize outside the workspace. `path =
  "../sibling"` → `E_PATH_ESCAPE`.

Same workspace-local invariant the resolver enforces for path deps —
vendor cannot be used as a side-channel to copy arbitrary host bytes
into an install repo.
