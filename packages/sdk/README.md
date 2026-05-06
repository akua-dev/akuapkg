# @akua-dev/sdk

TypeScript SDK for [akua](https://github.com/cnap-tech/akua). Every verb runs in-process via a bundled native addon (napi-rs) — same `akua-core` the CLI uses, no `akua` binary on `$PATH` required.

## Install

```sh
bun  add @akua-dev/sdk
pnpm add @akua-dev/sdk
npm  install @akua-dev/sdk
```

Node 22+ / Bun 1.3+. Browser target deferred to v0.2.x (the napi addon is host-side; a `wasm32-unknown-unknown` bundle is the path forward — see [docs/spikes/engines-on-wasm32-unknown-unknown.md](../../docs/spikes/engines-on-wasm32-unknown-unknown.md)).

`bun add` resolves the right per-platform binary via `optionalDependencies` on `@akua-dev/native-{darwin,linux,win32}-*`. The meta package is `@akua-dev/native`; the SDK depends on it transitively.

## Usage

```ts
import { Akua, AkuaUserError, AkuaRateLimitedError } from '@akua-dev/sdk';

const akua = new Akua();

const yaml = await akua.renderSource('package.k', PACKAGE_K_SOURCE, { replicas: 3 });
const lint = await akua.lint({ package: './package.k' });
const tree = await akua.tree({ workspace: '.' });
const summary = await akua.render({ package: './package.k', out: './deploy' });
```

Every method returns a typed result validated against a JSON Schema generated from the same Rust `serde` types the CLI emits. Contract drift throws at the parse boundary, not as `undefined.field` later:

```ts
try {
  await akua.render({ package: './package.k', out: './deploy' });
} catch (err) {
  if (err instanceof AkuaRateLimitedError) backoff();
  else if (err instanceof AkuaUserError) console.error(err.structured?.code);
  else throw err;
}
```

## Examples

Runnable recipes in [`examples/`](examples/):

```sh
bun run packages/sdk/examples/01-render-source.ts
bun run packages/sdk/examples/02-lint-package.ts
bun run packages/sdk/examples/06-diff-renders.ts
```

## Types + schema are derived, not hand-written

- `src/types/*.ts` — per-type TS from `ts-rs` derives on Rust serde types in `akua-core` + `akua-cli`.
- `src/schemas/akua.json` — a single bundled JSON Schema from `schemars`. Polyglot consumers (Python, Go, agents) validate against the same shape.

Drift is guarded by `task sdk:check` — regenerate + `git diff --exit-code`.

## Repo tasks

```sh
task sdk:gen             # regenerate types + schema from Rust
task sdk:check           # regenerate + diff-check (wired into `task ci`)
task sdk:build           # bun bundle + tsc declarations → packages/sdk/dist/
task sdk:test            # bun test (uses the bundled native addon)
task sdk:publish:check   # npm pack --dry-run
```

## Release flow

SDK versions track the Rust workspace version: one `v<semver>` tag drives a single unified `release.yml` workflow that builds the wasm bundle once, fans out into the native + cli matrices in parallel, then chains npm publishes (engines → per-platform native → meta-native → SDK) via job-level `needs:` dependencies. No npm polling, no inter-workflow races.

1. Land changes on `main`; `task ci` must be green.
2. Bump versions in `Cargo.toml`, `crates/akua-napi/package.json`, `packages/sdk/package.json`, all `crates/akua-napi/npm/<platform>/package.json`, `crates/akua-native-engines-npm/package.json`. Commit `release: vX.Y.Z`.
3. Tag `v<semver>` and push. The single `release.yml` workflow handles everything (npm + GitHub Release + Docker + Homebrew). Prerelease tags like `v<x.y.z>-rc1` publish to the npm `next` dist-tag and are marked as prereleases on GitHub.

See [`.github/workflows/release.yml`](../../.github/workflows/release.yml) for the full job graph.

## Still coming

- Browser target — bundler-build path requires `helm-engine-wasm` / `kustomize-engine-wasm` to compile to `wasm32-unknown-unknown` (currently `wasm32-wasip1` only). See [docs/spikes/engines-on-wasm32-unknown-unknown.md](../../docs/spikes/engines-on-wasm32-unknown-unknown.md).
- Engine `.wasm` deduplicated across platform packages via a single `@akua-dev/native-engines` package — currently each per-platform addon embeds its own copy.
