# akua hero — static frames handoff

> Historical artifact: the original designer's brief and statics. The
> motion piece evolved from here (now 7 scenes, 58 s, embedded video in
> scene 4 instead of a placeholder GIF). For current paths and the
> production pipeline see [`../../README.md`](../../README.md).

Six 1920×1080 PNG frames for the ~30s README hero motion piece, plus the
source HTML they were rendered from.

```
01-open.png              ~1.5s   wordmark + the claim
02-pipeline-today.png    ~3.5s   9 binaries, today
03-the-collapse.png      ~2.0s   9 tools → one binary
04-live-cli.png          ~5.0s   real CLI surface — placeholder for hero.gif
05-compiled-gitops.png   ~4.0s   render at CI, apply at the cluster
06-close.png             ~2.5s   Typed. Signed. Sandboxed. akua.dev
```

Each scene was designed for ~28-32px+ effective height at GIF-thumbnail
resolution, so the static fallback reads on github.com / npm / crates.io
without depending on motion. Page chrome (registration ticks, build stamps,
scene/06 markers) deliberately stripped — the document grain works for a deck,
but argues against the time pressure of a 30-second loop.

---

## Visual system

Single typeface, three accents, ≤3 information atoms per scene.

### Typography
**JetBrains Mono** for everything — display, body, terminal, the wordmark.
Weights in use: 400 (body), 500 (display sans), 600 (emphasis), 700 (wordmark
+ the giant numeric on Scene 2).

| element                      | size      |
|------------------------------|-----------|
| Scene 1 wordmark             | 460px     |
| Scene 6 invariant words      | 240px     |
| Scene 6 / Scene 3 sub-mark   | 128–560px |
| Scene 2 numeral "9"          | 640px     |
| Scene title (S2 kicker)      | 32px      |
| Scene headline (S3/S4/S5)    | 56–80px   |
| Scene 1 tagline              | 88px      |
| Scene 2 list / S5 step name  | 40–52px   |
| Caption / sub                | 28–36px   |
| Step label (S5 lab)          | 18–20px   |

Display lines (headlines, marks, the 9) are the dominant element on every
scene. Supporting type stays ≥28px so it survives thumbnail rendering.

### Color (CSS vars)
```
--bg-0      #0b0d10    deep ink (base)
--bg-1      #11151b    panel (terminal frame, S2 list bg)
--bg-2      #171c24    raised

--ink-0     #ECE9E1    primary text, warm paper
--ink-1     #b8bcc4    secondary
--ink-2     #7a8089    muted, sub-marks
--ink-3     #4a5057    whisper / step labels

--rule      #232a33    panel border / S6 foot divider

--typed     #b794f4    violet — Scene 1 pip 1, Scene 5 KCL, Scene 6 word 1
--signed    #5fd9a3    mint   — caret, OK ticks, Scene 5 OCI, Scene 6 word 2
--sandbox   #f0a85a    amber  — Scene 5 akuapkg render, Scene 6 word 3
--friction  #e87a5c    coral  — the big "9" on Scene 2 (the old way)
```

Each invariant is bound 1:1 to a color. Violet=types, mint=signatures,
amber=sandbox. Friction coral fires only on Scene 2's `9` — the friction
is the cardinality.

---

## Frame-by-frame motion notes

### 01 — Open `~1.5s`
**Purpose:** in one second, the viewer knows what this thing is called and
what it claims.

- Wordmark `akua` wipes in L→R (~250ms, cubic-out). Mint caret beat lands
  with the `a` and blinks once.
- Tagline `Typed. Signed. Sandboxed.` — each word + its colored pip fades
  in in sequence (~120ms apart). Pip leads its word by ~30ms.
- Sub fades in last.

**Loop seam:** Wordmark stays anchored — Scene 6's bottom-left wordmark
matches its weight and tracking so a continuous-cut loop snaps cleanly.

### 02 — Pipeline today `~3.5s`
**Purpose:** the viewer recognizes their own workflow. Nine real tools.

- Kicker `YOUR KUBERNETES DEPLOY, TODAY` types in.
- Coral `9` counts up from `0 → 9` over ~600ms with a slight overshoot.
- The right-column list builds top-to-bottom. Each tool line fades in
  with a ~80ms stagger so the eye reads "1, 2, 3, …" not "all-9-at-once".
- The footer line (`binaries · configs · failure modes`) types in last.

**Hold:** ~2.2s once the list is fully visible. A lot to scan; don't rush.

### 03 — The collapse `~2.0s`
**Purpose:** the payoff. Nine becomes one.

- Carry the `9` from Scene 2 — it scales up, dimes its coral, and morphs
  into the `akua` wordmark in cream. Single 500ms transform.
- The headline `nine tools → one binary.` fades in above; the `→` is the
  one mint accent in the line and arrives last with a small pop.
- Tri-color stripe (violet/mint/amber thirds) wipes in vertically along
  the left edge of the wordmark over ~400ms — that's the three-invariants
  preview for the close.
- Footer `same inputs · same lockfile · same hash` fades in.

**Hold:** ~1.0s.

### 04 — Live CLI `~5.0s`
**Purpose:** show, don't tell. This is a package manager surface, not a
templating tool.

**You'll replace the static body with `docs/hero.gif`.** The frame is
designed around that:
- Terminal panel chrome (traffic lights, path bar, "recorded — replace
  with hero.gif" hint) stays static; the body region is where the GIF
  drops in.
- One headline only (top of frame). No side annotations — the motion-piece
  version of those concepts is the syntax highlighting inside the terminal:
  violet for type names (`nginx.Values`), mint for verified states
  (`cosign · ECDSA P-256 · rekor#…`), amber for numerics (`148 fields`).

If you want a beat-by-beat caption overlay, hold one line per ~1.2s as
each command finishes:
- `$ akuapkg add nginx@18.2.0` → "typed schema generated at fetch"
- `$ akuapkg lock` → "every dep pinned, every signature verified"
- end → "no shell-out, no $PATH, no ambient filesystem"

### 05 — Compiled GitOps `~4.0s`
**Purpose:** the audit story. Cluster never sees KCL.

- Headline types in. The second clause `render at CI, apply at the cluster.`
  is muted ink — visually the sub-headline.
- Five-stage flow builds L→R. Each step name uses the accent color of the
  *thing it produces* (KCL=typed/violet, akuapkg render=sandbox/amber,
  signed OCI=signed/mint, PR diff & cluster stay cream — they're outcomes
  of the prior stages, not new properties).
- Each `→` arrow draws in between steps as the next step lands.
- The two-column foot (`render at CI · the cluster never sees KCL` and
  `apply at cluster · the diff is the audit`) fades in once the flow is
  complete. The mint `●` markers next to each label can pulse once.

### 06 — Close `~2.5s`
**Purpose:** the three invariants land.

- Three words stack-build with ~150ms stagger:
  `Typed.` (violet) → `Signed.` (mint) → `Sandboxed.` (amber).
- Each word fades in alongside its color pip. The trailing period of each
  word is part of the typography — JetBrains Mono renders it as a chunky
  square that visually pairs with the leading pip; the rhythm is
  `[pip] Word .` on every line.
- Wordmark `akua` wipes in from bottom-left (same wipe as Scene 1).
- `akua.dev` URL fades in on the right with the hairline divider above
  drawing L→R.

**Hold + loop:** ~1.5s, then cut back to Scene 1. The wordmark in 06's
bottom-left occupies similar visual real estate to 01's wordmark, so a
straight-cut loop reads as the wordmark "expanding" back to full scale.

---

## Re-cutting from source

`source/akua-hero-frames.html` is the canonical source — open it in any
browser, arrow keys navigate, print-to-PDF gives one page per scene.

Each scene is a `<section>` directly inside `<deck-stage>` with a scene
class (`.s1`, `.s2`, …). The CSS at the top has a tight `:root` token
block — change `--bg-0`, `--ink-0`, `--typed/signed/sandbox`, etc. and
every scene picks it up.

To regenerate the PNGs at 1920×1080:
```js
document.querySelector('deck-stage').goTo(N);   // N = 0..5
// screenshot the <section> at its native size
```

All text is static HTML (no script-rendered content). The Scene 4
terminal body is hand-written to mimic real `akua` output — replace
inline or swap the whole `.term` for an `<img>`/`<video>` of `hero.gif`
once you have the recording.

---

## Things deliberately not included

- **No emoji.** The brief was clear.
- **No drawn icons.** Tools are referenced by name. If you want real
  vendor marks on Scene 2's list or Scene 5's flow (helm, argo, cosign,
  wasmtime), drop them in at the same x-height as the tool name.
- **No "AI agents" language anywhere visible.** Scene 4's annotation in
  the dense v1 mentioned it; this v2 strips it. The motion piece is for
  platform engineers; the agent-first contract is the README copy.
- **No registration ticks, build-stamp footers, scene-N/06 markers.**
  Those were the documentation-density v1 — removed for v2 motion piece.
  If you want them back as a deliberate frame chrome (e.g. a fixed lower
  third with scene index), I can re-add them as one element.

## Things to flag back

- Scene 4 terminal lines are plausible-but-invented. Verify against
  real `akuapkg add / akuapkg lock` output before final cut. Particularly:
  - the `rekor#48291103` log index format
  - `nginx.Values · 148 fields` truncation
  - `sha256:c9a4…d3f1` hash truncation length
- Scene 2 lists `verifyImages admission` as item 09 — that's the admission
  webhook, not a binary. If you'd rather show 9 actual *binaries* and
  treat admission as out of scope, swap line 09 for something like
  `helm-secrets` or `kpt fn` so the "9 binaries" framing is exact.
