# Security audit — 2026-05-29

Point-in-time security audit of the akua codebase, conducted as a multi-reviewer
pass across five dimensions: sandbox isolation, path traversal, supply-chain
integrity (signing/fetch/credentials), input parsing & code generation, and
secrets handling. Findings are rated and cross-validated; the most severe finding
(`publish` not stripping `replace`) was independently confirmed by two reviewers.

This is a dated snapshot, not a living spec — see [security-model.md](security-model.md)
for the design invariants. Remediation status is tracked inline.

## Summary

No **sandbox-escape** (host file read/write, code execution, or network from
untrusted Package KCL or a declared malicious chart) was found — the wasmtime/WASI
isolation, the path-escape guard (`resolve_in_package` / `validate_workspace_path`,
canonicalize-then-contain), and the no-shell-out invariant all hold with evidence.
Cosign verification is cryptographically sound and fail-closed *once a key is
configured*; digest-pinning is the universal, verified-before-write integrity gate.

The actionable findings cluster in three areas: (1) a supply-chain **integrity**
gap (`publish` ships `replace` directives in the signed artifact), (2) **codegen
injection** from a malicious chart's `values.schema.json` into generated KCL, and
(3) **resource-exhaustion / DoS** hardening (uncapped engine Stores, unbounded
fetch buffering, decompression bombs).

| # | Severity | Finding | Area | Status |
|---|---|---|---|---|
| 1 | HIGH | `akua publish` does not strip `replace` directives before packing+signing | supply-chain | ✅ fixed |
| 2 | HIGH | KCL injection via unsanitized `values.schema.json` property names | codegen | ✅ fixed |
| 3 | HIGH | KCL docstring breakout via unescaped `"""` in schema descriptions | codegen | ✅ fixed |
| 4 | MEDIUM | Native helm/kustomize engine Stores have no memory cap + infinite epoch → chart DoS | sandbox | ✅ fixed |
| 5 | MEDIUM | Unbounded HTTP response buffering (OCI/helm/git) → OOM DoS | fetch | ✅ fixed |
| 6 | MEDIUM | Decompression bomb on gzip→tar layer/chart extraction | fetch | ✅ fixed |
| 7 | MEDIUM | `BasicAuth` derives `Debug`+`Serialize` with no redaction (latent secret leak) | secrets | ✅ fixed |
| 8 | MEDIUM | gix git transport honors ambient `GIT_SSL_NO_VERIFY` (first-resolve MITM window) | fetch | ✅ fixed |
| 9 | MEDIUM | Cosign verification is opt-in, contradicting "verify by default on pull" | supply-chain | ✅ docs reconciled |
| 10 | LOW | UTF-8 byte-slice panic on registry error body (`&body[..300]`) | parsing | ✅ fixed |
| 11 | LOW | Helm-repo `index.yaml` can downgrade `.tgz` download to `http://` | fetch | ✅ fixed |
| 12 | LOW | Helm-repo `chart` name path-joined without `..` validation (self-targeting) | path | ✅ fixed |
| 13 | LOW | git checkout trusts gix tree-entry filenames for `dest.join` (defense-in-depth) | path | ✅ fixed |
| 14 | LOW | `vendor add --json` `source_ref` echoes raw (uncanonicalized) git URL | secrets | ✅ fixed |
| 15 | INFO | OCI deps not validated for embedded `user:pass@` (symmetry with git/helm) | secrets | ✅ fixed |
| 16 | INFO | Worker preopen doc comment says "writable" but dirs are read-only (stale) | sandbox | ✅ fixed |
| 17 | INFO | `--timeout` not propagated into the worker epoch deadline | sandbox | ✅ fixed |

## Remediation status

**All 17 findings are remediated.** Findings 1-8 and 10-15 were fixed the same day,
each in an isolated worktree (disjoint file sets) and merged to `main` as the
`fix(security): …` commit series. **#9** was resolved by reconciling the docs: the
"verify by default" invariant in CLAUDE.md now states accurately that digest-pinning
is the universal verified-before-write gate and cosign signature verification engages
(fail-closed) when a `[signing].cosign_public_key` is configured — keeping signing
opt-in rather than breaking every key-less workspace. **#16** (stale worker preopen
comment) and **#17** (`--timeout` now derives the worker epoch deadline, with a unit
test) are fixed. All `akua-core` + `akua-cli` tests pass on the merged result; the CLI
builds clean. Each "Fix:" note below describes the change that landed.

## Findings

### 1. [HIGH] `akua publish` does not strip `replace` before signing
`crates/akua-core/src/package_tar.rs:133-144`, `crates/akua-cli/src/verbs/publish.rs:183`

CLAUDE.md: *"`akua publish` strips every `replace` directive from the artifact's
manifest before signing — consumers never inherit a publisher's replace."* This is
**not implemented.** `pack_workspace_with_vendored_deps` appends `akua.toml`
byte-for-byte, so the manifest's `replace` directives survive into the digested,
cosign-signed, SLSA-attested tarball. A consumer who pulls and renders in a
non-agent context (no `AKUA_REJECT_REPLACE`) inherits the publisher's `replace`.
Mitigations: the consumer's resolver canonicalizes `replace.path` under the
*consumer's* workspace and rejects `..`/absolute escape (so no host-file read), and
production/agent renders fail closed on replace. But the signed-artifact invariant
is violated. **Fix:** clear `replace` on every dep in the in-memory manifest and
re-serialize before it enters the tarball; add a publish test asserting the artifact
manifest is replace-free.

### 2 & 3. [HIGH] KCL codegen injection from `values.schema.json`
`crates/akua-core/src/values_schema.rs:207` (property names), `:333-347` (descriptions)

akua generates a typed KCL `schema Values` from a chart's `values.schema.json`.
Two untrusted strings are emitted into KCL source without escaping:
- **Property names** are emitted verbatim (`{prop_name}: {ty}`); only the *nested
  schema name* is pascal-cased. A crafted key (newline + a fabricated field or
  `check:` block) injects statements into `schema Values`.
- **Descriptions** are wrapped in `"""…"""` with no escaping of an embedded `"""`,
  so a description can close the docstring and append arbitrary KCL.

Reachability: the generated module is written to disk and `import charts.<name>`'d
into the render. The render runs in the wasmtime sandbox, so the ceiling is
**silent incorrect-but-trusted output, render DoS, or tampering with values fed to
the helm engine** — not host escape. Still a trust-model break: a third-party chart
injects KCL the consumer never wrote into signed output. **Fix:** validate property
names against the KCL identifier grammar (reject non-identifiers); escape/neutralize
`"""` in descriptions (or emit them as `#` comments). Test at the codegen-string
level (don't rely on the downstream parser — valid-but-injected KCL passes it).

### 4. [MEDIUM] Native engine Stores uncapped → chart/overlay DoS
`crates/engine-host-wasm/src/lib.rs:292-303`, callers `crates/akua-core/src/{helm,kustomize}.rs`

The render-worker Store is bounded (256 MiB memory `StoreLimits`, ~6 s epoch), but
the helm/kustomize engine Sessions are created with `Store::new(...)` and **no
memory limiter** and `set_epoch_deadline(i64::MAX)`, with no host-side wall-clock
around the engine call. A declared malicious chart with pathological templating can
consume CPU and up to ~4 GiB on a shared host. Not an escape (engine has no FS/net),
but violates the "memory/CPU/wall-clock caps" half of the sandbox invariant. **Fix:**
apply a `StoreLimits` memory cap + finite epoch deadline to engine Sessions, reusing
the worker's existing ticker/limit wiring.

### 5 & 6. [MEDIUM] Unbounded fetch buffering + decompression bombs
`crates/akua-core/src/oci_transport.rs:289`, `oci_fetcher.rs:738-744`, `helm_repo_fetcher.rs:187-191`

HTTP responses (manifests, blobs, `index.yaml`, `.tgz`) are buffered whole into a
`Vec` with no size cap; the blob digest is verified *after* the full body is in
memory, so a malicious/MITM registry can OOM the host before any check. Separately,
`flate2::GzDecoder` → `tar::Archive::unpack` has no decompressed-size/entry-count
cap, and the verified digest is of the *compressed* bytes — a small valid-digest
gzip can expand to fill disk. **Fix:** cap manifest/blob/index/tgz downloads with a
counting reader that aborts past a ceiling; cap total unpacked size + entry count.

### 7. [MEDIUM] `BasicAuth` is `Debug`+`Serialize` without redaction
`crates/akua-core/src/host_auth.rs:34`

`BasicAuth { username, password }` derives `Debug` and `Serialize`. No live path
prints/serializes it today, but any `{:?}` on a containing struct (e.g. `VendorArgs`)
or an accidental serialize would emit the plaintext password. **Fix:** manual
redacting `Debug`; drop `Serialize` (the `--auth-file` round-trip uses a separate
serde shape).

### 8. [MEDIUM] gix honors ambient `GIT_SSL_NO_VERIFY`
`crates/akua-core/Cargo.toml:113-119`

The `blocking-http-transport-curl-rustls` gix feature applies Git-compatible TLS env
settings, including `GIT_SSL_NO_VERIFY`. On a poisoned environment, an attacker can
disable TLS validation and MITM the *first* `akua add` of a git dep (TOFU window);
subsequent fetches are protected by the lockfile commit pin. **Fix:** pin TLS config
in the gix client and ignore `GIT_SSL_NO_VERIFY`, or document that git deps require
a trusted environment on first resolve.

### 9. [MEDIUM] Cosign verification is opt-in, not "verify by default"
`crates/akua-cli/src/verbs/render.rs:482-498`, `verify.rs:280-285`, `oci_fetcher.rs:439-449`

Signature/attestation verification only engages when `[signing].cosign_public_key`
is configured; absent a key, the crypto verify is a silent no-op and only
digest-pinning remains. The design *is* fail-closed once opted in (missing `.sig`
with a configured key is a hard error; `strict_signing` defaults true). The gap is
docs-vs-implementation. **Action:** reconcile — either document "verification engages
when a key is configured; digest-pin is the universal gate," or make a configured key
part of the default `strict_signing` posture.

### 10. [LOW] UTF-8 slice panic on registry error body
`crates/akua-core/src/oci_transport.rs:278-282`

`&body[..300]` panics if a multibyte char straddles byte 300; a registry returning a
>300-byte error body with a boundary-straddling char triggers it. **Fix:** use a
char-boundary-safe truncation.

### 11-15. [LOW/INFO] Remaining hardening
- **11** `resolve_tarball_url` passes through `http://` `.tgz` URLs from `index.yaml`
  (scheme downgrade; mitigated by digest pin). Reject `http://` when the repo was
  fetched over `https://`. (`helm_repo_fetcher.rs:136-138`)
- **12** Helm-repo `chart` name is `dest.join`'d without `..` validation
  (`helm_repo_fetcher.rs:181`, `mod_file.rs:109`); self-targeting. Reject path
  separators / `..` in `chart` at manifest validation.
- **13** git checkout `dest.join(entry.filename)` trusts gix tree-entry names
  (`git_fetcher.rs:471-485`); gix validates these, but add an explicit
  single-component guard (defense-in-depth).
- **14** `vendor add --json` `source_ref` uses the raw git URL (`vendor.rs:766`)
  rather than `canonicalize_url`; mitigated by the manifest userinfo guard. Apply
  `canonicalize_url`.
- **15** OCI dep URLs aren't run through `url_has_userinfo` at validation
  (`mod_file.rs`, OCI arm) — add for symmetry with git/helm.

## Verified safe (coverage)

- **No sandbox escape / no shell-out:** fresh per-render wasmtime Store, read-only
  WASI preopens only (no writable host dir), no `std::process`/subprocess in the
  render path, engines are wasm not shell-outs, host↔guest bridge passes only
  guest-memory offsets with `catch_unwind` panic isolation, no network in the worker.
- **Path-escape guard sound:** `validate_workspace_path` + `resolve_in_package`
  canonicalize-then-`starts_with`; absolute/`..`/symlink escape rejected; verified
  empirically against non-existent and partially-existing escape targets. Tar
  extraction retains the `tar` crate's `..`/absolute rejection (`set_overwrite` does
  not disable it). git checkout skips symlinks + submodules.
- **`reject_replace` / `offline` propagate into nested `pkg.render`** (the keystone
  fix): a sub-package cannot open an escape the root forbade; child path-deps bound
  to the child workspace.
- **Cosign crypto fail-closed (once keyed):** P-256 SPKI parse, payload
  `docker-manifest-digest` correlated to the fetched digest before accept, DSSE PAE
  binding prevents cross-type substitution, every error path returns `Err`.
- **Digest pinning verified-before-write:** OCI blob + lockfile digest both checked
  before extraction; helm `.tgz` sha256 checked before extraction; git commit pin
  fails hard on moved tags. SLSA subject re-checked against the lockfile pin.
- **Credentials:** prefix-confusion closed (`example.com` ∌ `example.com.evil.com`),
  userinfo stripped from lockfile URLs, `user:pass@` rejected at parse for git/helm,
  no credentials logged or in tracing spans, cosign private key in-memory only,
  `Credentials`/`CredsStore` not `Serialize`, `.akua/cache` gitignored. (Note: OCI
  auth *does* read ambient `~/.docker/config.json` / `auth.toml` by design — a
  separate, documented surface from the host-keyed git/helm path.)
- **Parsing:** YAML multidoc rejects alias bombs + deep nesting without panic; TOML
  manifest/lockfile parsing returns `Err` (no panic); `kcl_ident` collisions caught
  by `detect_ident_collisions`; lockfile source strings are formatted, never
  re-parsed into fetch instructions.

## Method & caveats

Five independent read-only reviewers, each adversarial within its dimension
(attempting to construct exploit paths, not just pattern-match), distinguishing
*confirmed* (code-read/reproduced) from *suspected*. This is not a substitute for a
funded external pentest or fuzzing campaign; it is a structured internal review of a
pre-alpha codebase. Findings 1 and 4-6 (DoS) assume an adversary who can get a
malicious chart/manifest into the dep graph or MITM a first-resolve fetch.
