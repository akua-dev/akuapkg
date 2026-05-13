# Compact tape — embedded inside the hyperframes composition.
#
# Five beats showing the package-manager surface, deterministic render,
# and ecosystem diff. Slow typing + generous reads so the demo is
# legible at thumbnail size. Workdir is examples/01-hello-webapp so the
# render step produces real K8s YAML, not a ConfigMap toy.

Output "hero-clip.gif"

Set Shell "bash"
Set FontSize 20
Set Width 1280
Set Height 720
Set Theme "Catppuccin Mocha"
Set TypingSpeed 34ms
Set Padding 28

Hide
Type "unset AGENT CLAUDECODE CURSOR_CLI GEMINI_CLI AKUA_AGENT && export AKUA_NO_AGENT_DETECT=1"
Enter
Type "cp -r /Users/robin/Developer/cnap-tech/akua/examples/01-hello-webapp $(mktemp -d)/blog && cd $_ && clear"
Enter
Sleep 400ms
Show

# Beat 1 — add a real Helm chart from OCI as a typed dep.
Type "akua add --oci oci://ghcr.io/stefanprodan/charts/podinfo --version 6.7.1 podinfo"
Sleep 300ms
Enter
Sleep 3000ms

Type "clear"
Sleep 150ms
Enter
Sleep 150ms

# Beat 2 — lockfile pinned by content-addressed digest.
Type "akua lock"
Sleep 250ms
Enter
Sleep 3200ms

Type "clear"
Sleep 150ms
Enter
Sleep 150ms

# Beat 3 — typed dep graph.
Type "akua tree"
Sleep 250ms
Enter
Sleep 3500ms

Type "clear"
Sleep 150ms
Enter
Sleep 150ms

# Beat 4 — sandboxed deterministic render. Output: real K8s manifests
# plus the deterministic sha256 of the rendered output.
Type "akua render --inputs inputs.yaml --out ./deploy"
Sleep 250ms
Enter
Sleep 4000ms

Type "clear"
Sleep 150ms
Enter
Sleep 150ms

# Beat 5 — materialize the typed OCI dep into the workspace's vendor tree.
# Output shows the cosign-verified digest, on-disk path, and byte size —
# proof the dep was fetched, content-addressed, and pinned without `helm`.
Type "akua vendor add podinfo"
Sleep 300ms
Enter
Sleep 6000ms
