import akua.ctx
import charts.nginx as c

# Sub-package: renders a local Helm chart via the typed `charts.nginx`
# alias. This package is composed by the root via `pkgs.webserver`; the
# chart resolver propagates the `nginx` dep context into this render.
schema Input:
    """Inputs for the webserver sub-package."""
    namespace: str = "demo"
    """Kubernetes namespace forwarded to the Helm release."""

input: Input = ctx.input()

resources = c.template(c.TemplateOpts {
    values = {}
    release = "web"
    namespace = input.namespace
})
