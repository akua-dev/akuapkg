# Renders a chart vendored into `.akua/vendor/upstream/` via
# `akuapkg vendor add upstream`. The dep declaration in `akua.toml`
# stays canonical (`path = "./upstream-chart"`); the resolver
# prefers the vendored copy when present, so render is offline-
# safe once the vendor tree is committed alongside the Package.

import akua.ctx
import akua.helm

schema Input:
    """Public inputs for the vendored upstream."""

    message: str = "rendered from .akua/vendor/upstream"
    """Embedded into the rendered ConfigMap so you can spot which copy
    of the chart was used."""

input: Input = ctx.input()

_manifests = helm.template(helm.Template {
    chart     = ".akua/vendor/upstream"
    values    = {
        message = input.message
    }
    release   = "vendored"
    namespace = "default"
})

resources = _manifests
