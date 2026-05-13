# Hero motion piece — production source

The 58 s README hero (`docs/assets/hero.mp4` + GIF fallback) is built
from sources in this directory. Self-contained so it can be re-rendered,
re-cut, or re-themed without re-deriving the inputs.

```
hero-source/
├── Taskfile.yml         pipeline (bun + vhs + ffmpeg)
├── hyperframes/         HTML/CSS/GSAP composition (6 scenes, 58 s)
├── vhs/                 charm.sh/vhs tape — terminal session for scene 4
└── design/              Claude Design handoff: brief, statics, tokens
```

## Run the pipeline

```sh
cd docs/hero-source
task install     # one-time: `bun install` inside hyperframes/
task all         # tape → clip-mp4 → hyperframes render → gif
```

Outputs land in `docs/assets/{hero.mp4, hero.gif, hero-clip.mp4, hero-clip.gif}`.

Individual steps: `task tape`, `task clip-mp4`, `task render`, `task gif`,
`task preview` (live-reload in browser), `task lint`. `task --list`
shows everything.

## Required tools

| Tool | Purpose |
|---|---|
| [bun](https://bun.sh) | package manager + script runner for hyperframes |
| [charm.sh/vhs](https://github.com/charmbracelet/vhs) | terminal recorder |
| [ffmpeg](https://ffmpeg.org) | GIF↔MP4 conversion + palette dither |
| `akua` itself | the tape runs real commands against `examples/01-hello-webapp` |

## The two pipelines

1. **`vhs/hero-clip.tape`** records a real `akua` session — typed Helm
   dep → digest-locked lockfile → typed tree → sandboxed render → vendor
   materialization. Outputs `hero-clip.gif` (standalone) and
   `hero-clip.mp4` (which scene 4 of the composition embeds as a
   timeline-scrubbable `<video>` so the demo advances with the rest of
   the piece instead of looping at screenshot rate).

2. **`hyperframes/index.html`** is a 1920×1080 composition rendered to
   MP4 via [HeyGen's hyperframes](https://github.com/heygen-com/hyperframes).
   Six scenes:

   1. Open — wordmark + `Typed Signed Sandboxed` tagline.
   2. The 9-tool pipeline today (helm, kustomize, kyverno, syft, cosign
      sign + attest, docker buildx, argocd, verifyImages).
   3. Nine → one. Tri-color invariant stripe lands.
   4. Live CLI — the embedded `hero-clip.mp4` plays inside a terminal
      frame. **Centerpiece**, 30 s of the 58 s runtime.
   5. Compiled GitOps: KCL Package → akua render → signed OCI →
      PR diff → admission re-verify.
   6. Agent-first: detected agents + `--json` / typed exit codes /
      9 skills shipped.
   7. Close — three giant invariants + `akua.dev`.

## Iterating on visuals

`hyperframes/index.html` is one self-contained file. Tokens at the top
of the `<style>` block:

```
--bg-0   #0b0d10     deep ink
--ink-0  #ECE9E1     warm paper
--typed  #b794f4     violet — typed invariant
--signed #5fd9a3     mint   — signed invariant
--sandbox #f0a85a    amber  — sandboxed invariant
--friction #e87a5c   coral  — the friction-9 on scene 2
```

Change once, re-render. Per-scene motion notes (hold times, easing,
loop-seam logic) live in `design/handoff/README.md` — these were
written by the designer and the GSAP timeline in `index.html`
implements them.

## Iterating on the CLI demo

Edit `vhs/hero-clip.tape`, then `task tape`. The tape:

- Copies `examples/01-hello-webapp` to a fresh `$(mktemp -d)/blog` so
  every recording starts from a known state.
- Clears `AGENT*` / `CLAUDECODE` envs and sets `AKUA_NO_AGENT_DETECT=1`
  so the output is human-readable, not JSON.
- Uses 34 ms/char typing and 3–6 s reads between commands. Slow on
  purpose — viewers have to read the output at thumbnail scale.

After re-rendering the tape, run `task clip-mp4` (or just `task all`)
to refresh the embedded MP4 the composition uses.

## Original brief

`design/brief.md` is the prompt that produced the visual system at
`design/handoff/` from Claude Design. If the design ever needs a full
re-cut, hand that brief plus the current state of the blog post
(`apps/web/src/content/blog/akua-introduction.svx`) back to a designer.

## How the assets get into the README

`README.md` at the repo root uses the image-linked-to-MP4 pattern
(GitHub strips `<video>` tags from rendered markdown):

```html
<a href="docs/assets/hero.mp4">
  <img src="docs/assets/hero.gif" alt="…" width="900" />
</a>
```

The GIF plays inline on every renderer; clicking opens GitHub's
built-in player on the MP4. The MP4 is the higher-fidelity reference.
