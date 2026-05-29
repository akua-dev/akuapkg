# akua/pkg — typed wrapper over the `kcl_plugin.pkg` surface.
#
# Users:
#
#   import akua.pkg
#
#   resources = pkg.render(pkg.Render {
#       package = "upstream"
#       inputs  = { name = "frontend" }
#   })
#
# `package` is the dep alias declared in this Package's `akua.toml`
# `[dependencies]`. The host's resolver maps it to the upstream's
# directory; user code never writes a filesystem path (CLAUDE.md "No
# filesystem paths in user-authored KCL."). There is no `path = "..."`
# escape hatch: cross-Package composition goes through typed aliases.
#
# Options-schema call style matches `akua.helm` / `akua.kustomize`. The
# underlying `kcl_plugin.pkg.render` host plugin loads + renders the
# inner Package synchronously and returns the resulting list — so
# list-comprehension patches and filter expressions on the result work
# natively.

import kcl_plugin.pkg as _pkg

schema Render:
    """Options for a single nested-Package render."""

    package: str
    """Dep alias from this Package's `akua.toml` `[dependencies]`. The
    host resolves it to the upstream's `package.k`."""

    inputs: {str:} = {}
    """Inputs forwarded to the inner Package's `ctx.input()`. The
    upstream's `Input` schema validates field names + types on the
    inner side. Inside `pkgs.<alias>.render`'s lambda body KCL also
    coerces a typed `upstream.Input { ... }` to this shape — same
    rule helm.template uses for `values: {str:}` accepting typed
    `Values` schemas."""

# Render a nested Package.k and splice its `resources` list into the
# caller's resources. The schema itself is passed through to the
# plugin, which reads fields by name.
render = lambda opts: Render -> [{str:}] {
    _pkg.render(opts)
}
