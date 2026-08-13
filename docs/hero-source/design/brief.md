# Brief — static mockups for the akua hero motion piece

You already have our design system. I need static mockups (one per scene, 1920×1080, PNG or SVG) for a ~30-second motion piece that ships at the top of github.com/cnap-tech/akua's README. Hand them off as a zip; I'll do the animation, build, and final render.

Your job is to design the static frames. My job is to make them move. Don't constrain the visual language — pick the language that fits the meaning. The notes below are about **what each scene means**, not what it should look like.

## What akua is

akua is the bun/deno collapse for cloud-native packaging. One Rust binary that covers package manager, runtime, formatter, linter, test runner, REPL, dev loop, and signed-OCI publisher. The current state of cloud-native packaging in 2026 is what JavaScript was in 2014: nine single-purpose tools held together by CI glue nobody wants to own. We replaced the glue with one binary.

Three invariants are wired into the runtime, not bolted on:

1. **Typed.** Inputs are KCL schemas with compile-time constraints. A typo fails at parse time, not at `kubectl apply`. (KCL is a CNCF Sandbox project; we don't own it, we embed it.)
2. **Signed.** Every `akuapkg publish` emits a cosign ECDSA P-256 signature plus a SLSA v1 attestation. Consumers verify on pull. Unsigned publishing is a flag, not a default.
3. **Sandboxed.** Every render runs inside a wasmtime WASI sandbox: no shell-out, no `$PATH` lookup, no ambient filesystem, no network. Untrusted Packages are safe to render on shared hosts.

Same inputs + same lockfile + same akuapkg version → byte-identical output. That determinism is load-bearing for the audit story (Compiled GitOps: render at CI, diff in the deploy repo is exactly what hits the cluster).

## Who this is for

Primary audience: **AI agents operating in Linux sandboxes** (Claude Code, Cursor, Codex, Gemini CLI, and the long tail). Every verb emits `--json`, uses typed exit codes, accepts `--plan` for dry-run. Skill manifests ship with the binary under `skills/`. Secondary audience: platform engineers reviewing what the agent did.

This is not a B2C product. The viewer is technical, suspicious of marketing language, and already knows what `helm template` and `cosign attest` are. They're trying to figure out in three seconds whether akua is serious or a toy.

## Voice we hold to (don't violate)

From our CLAUDE.md, things we refuse:
- Marketing-speak ("empower," "democratize," "unlock," "revolutionize," "AI-first")
- Emoji
- Feature-bingo
- Inventing akua-specific vocabulary when a standard exists (JSON Schema, OpenAPI, KRM, OCI, cosign, SLSA)

What we use instead: precise verbs, real tool names, real output shapes. Show, don't tell. The terminal beat in the motion piece shows real CLI output — not a fabricated mockup.

Brand wordmark is lowercase `akua`. Tagline: **Typed. Signed. Sandboxed.** Sub: *One binary. Every verb. Cloud-native packaging.*

## What the motion piece has to do

Convey, in 30 seconds, to a skeptical platform engineer scrolling past on github.com:

- The before state — the nine-tool pipeline they currently run (`helm template`, `kustomize build`, `kyverno test`, `syft`, `cosign sign`, `cosign attest`, `docker buildx`, `argocd sync`, `verifyImages` admission). Nine configs, nine failure modes.
- The collapse — these become one binary. One signed artifact per version, one Rekor entry per publish, one git diff per deploy.
- The CLI as living thing — actual terminal output from `akuapkg add` / `akuapkg lock` / `akuapkg tree`, showing Helm charts as typed deps with cosign-locked digests. (You'll get a real terminal recording to embed; design the frame around it.)
- The Compiled GitOps loop — Package authored in KCL → CI runs `akuapkg render` + `akuapkg publish` → signed OCI artifact lands in a deploy repo → ArgoCD / Flux syncs → admission re-verifies on apply. Render at CI, apply at the cluster.
- The three invariants as the close — Typed, Signed, Sandboxed.

The piece is the README hero on github.com. It autoplays muted, loops, and is the first thing anyone sees. It runs alongside a `<img>` fallback for npm/crates.io readers who don't get `<video>`.

## Scenes to mock up (purposes, not visuals)

Don't feel bound to exactly five frames — propose more or fewer if the story flows better. These are the beats I'd expect:

1. **Open.** Establish what the thing is called and what it claims. The viewer should know in one second whether to keep watching.
2. **The pipeline today.** Show that a real Kubernetes deploy runs ~9 distinct tools, each with its own config and version and failure mode. The viewer must recognize their own workflow here.
3. **The collapse.** Those tools become one runtime. This is the moment of payoff for the previous frame.
4. **Live CLI.** A terminal window showing real `akua` output (you'll receive a GIF of the actual session — design the surrounding frame, captions, annotations). The recording shows: scaffold → add a Helm chart as a typed dep → digest-locked lockfile → typed dep tree. Captions should reinforce that this is a package manager surface, not a templating tool.
5. **Compiled GitOps loop.** Author → CI render → signed OCI → ArgoCD/Flux → cluster admission. The reader should understand that the cluster never sees KCL; it sees rendered, signed YAML that a human read in a PR diff. This is the audit-story frame.
6. **Close.** The three invariants land. Typed. Signed. Sandboxed. URL: akua.dev.

If a single frame is doing two jobs, split it. If two frames are saying the same thing, merge.

## What I need back

- One static frame per scene, 1920×1080 PNG (or SVG if you have vector source).
- A short note per frame explaining the intended hold time, what enters/exits, and any text that should animate in vs appear instantly.
- The composition system / tokens you used (color, type scale, spacing) if they extend our design system anywhere.
- Source files if you have them (Figma export, Penpot, whatever) — not required, but lets me re-cut without re-prompting.

Package the lot as a single zip. I'll diff against the current rough cut and rebuild the animation in HTML/CSS/GSAP (we render with HeyGen's hyperframes, so it has to live in the browser).

## Source material to read before designing

- The introductory blog post (canonical voice — read this first): https://cnap.tech/blog/akua (or the local source at `apps/web/src/content/blog/akua-introduction.svx`). Particularly the sections "The pipeline your Kubernetes deploy actually runs," "We have seen this end before" (the bun/deno parallel), and "Three invariants we do not get from the CLI ecosystem."
- The README hero copy as it stands today: github.com/cnap-tech/akua
- The CLI contract docs: `docs/cli.md`, `docs/cli-contract.md`
- The current rough cut is a one-shot strawman built in HTML/CSS/GSAP rendered via hyperframes. Treat it as a thing to react against rather than a starting point — the structure is roughly right, the visual identity is not the final answer. Happy to share the source if useful, but assume you have not seen it.

## Constraints worth knowing

- GitHub renders `<video>` on github.com but not on npm / crates.io. The piece needs to read as a sequence of strong static frames so the GIF fallback isn't a step down — each scene should hold readably for ~half a second, not depend on motion to make sense.
- Loops. Don't end on a frame that fights starting over.
- Accessibility: text contrast must hold. Anything below 18px is risky on a phone-rendered README; design for the small-thumbnail case.
- No audio.

## What I'm not asking you to do

- Pick a visual style — that's your call.
- Write copy beyond what's already in the blog and tagline. If you propose new captions, flag them; we'll iterate.
- Animate. Hand off statics.

Time budget: whatever produces the best result. This is the hero of an open-source project that doesn't get a second first impression.
