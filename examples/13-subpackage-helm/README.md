# 13-subpackage-helm

> Composes an Akua sub-package that owns its own Helm chart dependency.
> The root package passes typed inputs down through `pkgs.webserver`
> while the sub-package resolves and renders its chart.

This example shows package composition across a local path dependency.
The root `package.k` imports `pkgs.webserver` and calls
`webserver.render(webserver.Input{...})`; the `webserver` sub-package
declares a Helm chart dependency in its own `akua.toml`. At render time,
Akua resolves the sub-package chart context and exposes it to the
sub-package implementation.

## What's here

| file | purpose |
|---|---|
| `package.k` | Root package that delegates rendering to `pkgs.webserver`. |
| `akua.toml` | Declares `webserver = { path = "./deps/webserver" }`. |
| `deps/webserver/` | Sub-package with its own Helm chart dependency. |
| `inputs.example.yaml` | Namespace input passed from the root to the sub-package. |
| `rendered/` | Reference output committed for integration tests. |

## Render

```sh
akuapkg render --out ./rendered
```

The interesting part is the import boundary: root package inputs remain
typed at the call site, and chart resolution stays local to the
sub-package that declared the chart.
