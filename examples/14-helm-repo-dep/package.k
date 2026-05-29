# Package that pulls podinfo from the classic HTTPS Helm repository at
# stefanprodan.github.io/podinfo and renders it with a typed values shape.
#
# `import charts.podinfo` resolves through the `podinfo` dep in akua.toml:
# the resolver fetches the versioned `.tgz` from the repository's index.yaml,
# pins its sha256 in akua.lock, and registers the unpacked chart as the
# `charts.podinfo` KCL module. No Helm binary required; no shell-out.
#
# Render (requires `akua add` online first to populate the cache):
#
#   akua render --out ./rendered
#
# The dep source is `repo`/`chart`/`version` rather than `oci` or `path` —
# the same resolver machinery, a different transport. Compare with example 01
# (local path dep) and example 10 (OCI dep) to see all three dep forms.

import charts.podinfo as p

resources = p.template(p.TemplateOpts {
    values = {
        replicaCount = 1
    }
})
