import akua.ctx
import pkgs.webserver as ws

# Root Package: composes the `webserver` sub-package, which itself
# renders a local Helm chart. Demonstrates that `charts.*` context
# propagates through `pkgs.<alias>` composition — a sub-package can
# declare its own Helm chart dep and have it resolved at render time.
#
# Render:
#
#   akua render --out ./rendered
schema Input:
    """Public inputs for the subpackage-helm root Package."""
    namespace: str = "demo"
    """Kubernetes namespace passed down into the sub-package's chart render."""

input: Input = ctx.input()

# Delegate entirely to the sub-package. The synthesized `pkgs.webserver`
# stub owns the `render` lambda; `ws.Input` type-checks here — typos
# surface as KCL compile errors, not runtime failures.
resources = ws.render(ws.Input {
    namespace = input.namespace
})
