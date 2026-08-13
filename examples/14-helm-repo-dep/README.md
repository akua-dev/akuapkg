# 14-helm-repo-dep

> Renders a Helm chart from a classic HTTPS Helm repository, pinned in
> `akua.lock` by chart version and tarball digest.

This example covers `repo` dependencies: charts published through an
`index.yaml` rather than OCI or a local path. `akuapkg add` resolves the
repository index, pins the selected chart archive digest in `akua.lock`,
and registers the chart as `charts.podinfo` for KCL rendering. Render
uses Akua's embedded Helm engine; no Helm binary or shell-out is needed.

## What's here

| file | purpose |
|---|---|
| `package.k` | Imports `charts.podinfo` and renders it with typed values. |
| `akua.toml` | Declares the `podinfo` chart from `https://stefanprodan.github.io/podinfo`. |
| `akua.lock` | Pins chart version `6.12.0` and the fetched archive digest. |
| `rendered/` | Reference output committed for integration tests. |

## Render

```sh
akuapkg render --out ./rendered
```

Classic Helm repositories are useful when an upstream chart has not
moved to OCI yet. The lockfile still gives the same reproducibility
contract: exact chart version, exact digest, repeatable render.
