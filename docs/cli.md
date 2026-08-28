# Akuapkg CLI reference

Reference for the standalone `akuapkg` binary. Akuapkg authors, renders, tests, and publishes cloud-native packages. It is distinct from the Akua platform CLI, whose binary is `akua`.

The platform CLI embeds Akuapkg under `akua pkg`. For example, these invocations run the same package command:

```sh
akuapkg render --inputs inputs.yaml
akua pkg render --inputs inputs.yaml
```

Use `akuapkg <command> --help` for the authoritative flags in the checked-out source. When Akuapkg is embedded, help and usage use the outer invocation, such as `akua pkg render --help`.

For the universal contract every verb honors (JSON output, exit codes, idempotency, plan mode, timeouts), see [cli-contract.md](cli-contract.md).

> This reference includes commands implemented by the current source. Run `akuapkg --help` against your installed version for its authoritative command list.
>
> **Shipped today (28 verbs):**
> `init` · `whoami` · `version` · `verify` · `render` · `add` · `vendor` · `dev` · `test` · `tree` · `pull` · `publish` · `sign` · `update` · `lock` · `push` · `repl` · `pack` · `remove` · `diff` · `check` · `inspect` · `lint` · `fmt` · `cache` · `auth` · `export` · `api`
>
> Run `akuapkg --help` at the command line for the authoritative live list.

---

## Top-level flags

These flags are accepted by every verb:

| flag | description |
|---|---|
| `--json` | emit structured JSON to stdout |
| `--plan` | compute what the command would do; do not write |
| `--timeout=<duration>` | max time before exit 6 (e.g. `30s`, `5m`) |
| `--idempotency-key=<uuid>` | safe-retry key for write operations |
| `--log=<text\|json>` | stderr log format (default: text) |
| `--log-level=<debug\|info\|warn\|error>` | filter logs |
| `--verbose` / `-v` | more detail in logs |
| `--help` / `-h` | help for this verb |
| `--describe --json` | machine-readable spec of this verb |
| `--no-color` | disable terminal colors (implicit under `--json`) |
| `--no-interactive` | never block on stdin; fail with exit 1 if input is missing (implicit in agent context) |
| `--no-agent-mode` | disable agent-context auto-detection for this invocation |

### Agent-context auto-detection

When `akuapkg` runs inside an AI-agent session, it detects this from environment variables and auto-enables `--json`, `--log=json`, `--no-color`, `--no-progress`, and `--no-interactive`. Detection is keyed off `AGENT=<name>` (standard), `CLAUDECODE`, `GEMINI_CLI`, `CURSOR_CLI`, or `AKUA_AGENT`. Explicit flags always override detection.

```sh
# Human shell — text output
akuapkg render
[pretty text output]

# Agent context — auto-JSON, no flag needed
CLAUDECODE=1 akuapkg render
{"format":"raw-manifests","target":"./deploy","manifests":3,"hash":"sha256:…"}
```

See [cli-contract.md §1.5](cli-contract.md#15-agent-context-auto-detection) for the full detection rules, override semantics, and env-var reference.

---

## `akuapkg init` ✅

Scaffold a new package or workspace.

```
akuapkg init [name] [flags]
```

Creates a directory with:
- `package.k` — typed KCL Package definition
- `inputs.example.yaml` — sample input
- `.akua/` — metadata + lockfile location
- `README.md` — minimal docs stub

### Flags

| flag | description |
|---|---|
| `--template=<name>` | use a template (see `akuapkg init --list-templates`) |
| `--package-name=<name>` | name for the Package (defaults to directory name) |
| `--no-git` | skip `git init` |
| `--list-templates` | list available templates |

### Templates

- `app` — single-service app (default)
- `app-with-db` — app + managed Postgres
- `umbrella` — multi-service composition
- `platform-std` — platform-team-published reusable package
- `empty` — bare package.k with a minimal schema

### Exit codes

0 success, 1 if target directory exists and is non-empty.

### JSON output

```json
{
  "name": "my-pkg",
  "path": "/absolute/path/my-pkg",
  "template": "app",
  "files": ["package.k", "inputs.example.yaml", ".akua/", "README.md"]
}
```

---

## `akuapkg add` ✅

Insert a dependency into `akua.toml`. Pure manifest edit — the resolver best-effortly updates `akua.lock` immediately after.

```
akuapkg add <name> (--oci=<url> | --git=<url> | --path=<path> | --repo=<url> --chart=<chart>) [flags]
```

Exactly one source flag is required. `--repo` requires `--chart`.

### Dependency sources

| source | flags | use when |
|---|---|---|
| OCI | `--oci=<url>` | published signed artifact (most common) |
| Git | `--git=<url>` | non-OCI-distributed sources |
| Path | `--path=<path>` | workspace-local, dev-only |
| Helm repo | `--repo=<url> --chart=<chart>` | classic HTTPS Helm repository |

### Examples

```sh
# OCI dep
akuapkg add cnpg --oci oci://ghcr.io/cloudnative-pg/charts/cluster --version 0.20.0

# Git dep pinned to a tag
akuapkg add tooling --git https://github.com/acme/tools --tag v1.2.3

# Local path dep
akuapkg add shared --path ../shared

# HTTPS Helm-repo dep
akuapkg add temporal --repo https://go.temporal.io/helm-charts --chart temporal --version 0.62.0

# Replace an existing entry
akuapkg add cnpg --oci oci://ghcr.io/cloudnative-pg/charts/cluster --version 0.21.0 --force
```

### Flags

| flag | description |
|---|---|
| `--oci=<url>` | OCI source URL (`oci://…`) |
| `--git=<url>` | Git source URL |
| `--path=<path>` | local filesystem path |
| `--repo=<url>` | HTTPS Helm-repo URL (pairs with `--chart`) |
| `--chart=<name>` | chart name within the Helm repo (required with `--repo`) |
| `--version=<version>` | version constraint; required for OCI and Helm-repo deps |
| `--tag=<tag>` | git tag (alternative to `--rev`) |
| `--rev=<sha>` | git commit SHA (alternative to `--tag`) |
| `--force` | replace an existing entry under `name` |
| `--workspace=<path>` | workspace root containing `akua.toml` (default: `.`) |

### Exit codes

0 success, 1 user error, 2 system error.

### JSON output

```json
{
  "name": "temporal",
  "source": "helm",
  "source_ref": "https://go.temporal.io/helm-charts",
  "version": "0.62.0",
  "replaced": false
}
```

---

## `akuapkg vendor` ✅

Materialize and inspect the workspace vendor tree at `.akua/vendor/`.

```
akuapkg vendor <subcommand> [flags]
```

Subcommands:
- `add <name>` — copy the declared dependency into `.akua/vendor/<name>/` and pin its digest in `akua.lock`. The dependency must already exist in `[dependencies]`; otherwise the command fails with a suggestion to declare it in `akua.toml`. Works for `path`, `oci`, `git`, and `helm` (repo) deps alike — the resolver's vendor-first lookup is universal across all four source kinds, so once added, the canonical source can be deleted and `akuapkg render` still succeeds via the vendored bytes.
- `check` — compare the on-disk vendor trees against `akua.toml` + `akua.lock`. Drift exits with code `1`.
- `list` — enumerate on-disk vendor trees, including orphaned entries.

`add` honors the universal write-contract flags: `--plan`, `--timeout`, and `--idempotency-key`. `check` and `list` are read-only.

### Auth flags (private git remotes)

`vendor add` accepts credentials at the call site for fetching private git deps. Akua never reads ambient credential files (`~/.netrc`, `~/.docker/config.json`, env vars) — the SDK and CLI surface are the only auth sources. See [E_MANIFEST_GIT_USERINFO](errors/E_MANIFEST_GIT_USERINFO.md) for why credentials in `akua.toml` URLs are rejected.

| flag | description |
|---|---|
| `--auth <prefix>=<user>:<password>` | Repeatable. Credential for a private git remote, keyed by URL prefix. The prefix is matched longest-first against the dep's URL — same rule git's credential helper uses. Example: `--auth akua-git.cnap.tech/org-A=org-A:token`. |
| `--auth-file <path>` | TOML file with a `[auth]` table keyed by URL prefix. `--auth` flags override file entries on conflict. The path must be explicit; akua never auto-discovers credential files. |

`--auth-file` shape:

```toml
[auth]
"akua-git.cnap.tech" = { username = "svc", password = "tok" }
"akua-git.cnap.tech/org-A" = { username = "org-A", password = "tokA" }
```

### Git HTTPS trust

All native Git fetch paths preserve the configured CA bundle from `GIT_SSL_CAINFO` or Git's `http.sslCAInfo`, including initial clones and cached-repository refreshes used by dependency resolution, `vendor add`, `pack`, and publish-time vendoring. Akua always forces certificate verification on: `GIT_SSL_NO_VERIFY` and `http.sslVerify=false` are ignored, so a CA bundle must actually validate the remote certificate. Only the CA bundle is copied from Git's HTTP transport configuration; ambient extra headers, proxy credentials, and other Git HTTP options are not forwarded by this trust setup.

Lockfile guarantee: regardless of the credential used to fetch, `akua.lock`'s `source` field stores the canonicalized URL with userinfo, default ports, and `.git` suffix stripped. Credentials never leak into `akua.lock`.

See `examples/12-vendor-offline/` for the end-to-end offline-render contract demonstrated against a path dep with the canonical source deleted.

---

## `akuapkg lint` ✅

Parse-only check of a `package.k` — catches syntax errors and import-
resolution failures without executing the program. Runtime errors
(schema validation, unresolved options, engine failures) surface
through `akuapkg render --dry-run`.

```
akuapkg lint [flags]
```

### Flags

| flag | description |
|---|---|
| `--package=<path>` | path to the `package.k` file (default `./package.k`) |

### Exit codes

0 clean, 1 parse errors (or user error), 2 system error.

### JSON output

```json
{
  "status": "ok",
  "issues": []
}
```

Or on parse failure:

```json
{
  "status": "fail",
  "issues": [
    {
      "level": "error",
      "code": "Error(InvalidSyntax)",
      "message": "invalid token '!', consider using 'not '",
      "file": "/abs/path/package.k",
      "line": 2,
      "column": 2
    }
  ]
}
```

> **Planned expansion (🚧).** The target surface also checks
> Regal-style Rego lints, policy-tier compatibility, cross-engine
> reference integrity, and offers `--fix` auto-format integration.
> Lands with the policy pipeline (Phase C).

---

## `akuapkg render` ✅

**Run the Package's program.** Evaluate the KCL, invoke every source engine (Helm, kro, Kustomize), compose results, produce deploy-ready manifests.

```
akuapkg render [path] [flags]
```

**Discovery.** With no `path`, renders every user-authored document in the workspace whose schema declares render semantics — typically the workspace's App-shaped documents that reference a Package and carry inputs. With a `path`, renders only that file. Users author their own App / Environment / etc. schemas (akua does not specify them; see [package-format.md](package-format.md)); `render` processes whichever documents the workspace declares as renderable.

> **Not the same as `akuapkg export`.** `render` executes the full pipeline against customer inputs and writes manifests a reconciler applies to a cluster. `export` converts a canonical artifact (schema, user-authored KCL document, policy bundle) into a format view (JSON Schema, YAML, OpenAPI, Rego bundle). Render needs inputs; export usually doesn't. Render invokes engines; export is format translation. See [`akuapkg export`](#akuapkg-export) below.

### Flags

| flag | description |
|---|---|
| `--package=<path>` | path to the `package.k` file (default `./package.k`) |
| `--inputs=<file>` | inputs file (JSON or YAML). When omitted, probes `./inputs.yaml` then `./inputs.example.yaml` next to the package; falls back to schema defaults if neither exists |
| `--out=<dir>` | write to directory (default: `./deploy/`) |
| `--summary-out=<file>` | also write the canonical `RenderSummary` JSON to a declared file; stdout is unchanged (incompatible with `--dry-run` and `--stdout`) |
| `--stdout` | print rendered manifests as multi-doc YAML to stdout instead of writing files |
| `--dry-run` | render but don't write files |

> **Engines.** Helm and Akua-package composition reach the user via alias-method calls — `webapp.template(webapp.TemplateOpts{values = webapp.Values{...}})`, `upstream.render(upstream.Input{...})` — synthesized per dep from `akua.toml`. Kustomize stays engine-direct (`kustomize.build({path = "./overlays"})`) because its input is a within-Package directory, not a typed dep. All backends ship as embedded WASM modules; akua never shells out to `helm` or `kustomize` binaries — every engine runs inside the wasmtime sandbox alongside the render worker. See [`docs/security-model.md`](security-model.md) and [`docs/embedded-engines.md`](embedded-engines.md).
>
> **One render output.** akua writes raw YAML manifests, one file per resource. Distribution shapes like Helm charts or OCI bundles are future `akuapkg publish --as <format>` concerns — they wrap rendered manifests at distribution time, not as a Package-declared output.

### Exit codes

0 success, 1 user error, 2 system error. (Phase B adds 3 for policy deny.)

### JSON output

```json
{
  "format": "raw-manifests",
  "target": "./deploy",
  "manifests": 1,
  "hash": "sha256:…",
  "files": ["000-configmap-hello.yaml"]
}
```

`format` is always `"raw-manifests"` today. `target` is the resolved output directory. `hash` is `sha256:<hex>` of the concatenated `<filename>\n<yaml>` blocks — stable across runs when inputs + lockfile + akuapkg version match.

Build systems should use `--summary-out=<file>` when the summary must be a declared output instead of parsing process stdout. The file contains this exact compact JSON contract plus a trailing newline, creates missing parent directories, and remains the bare `RenderSummary` even when `--debug` wraps JSON stdout. Human, JSON, and agent-selected stdout behavior is otherwise unchanged.

---

## `akuapkg diff` ✅

Structural diff between two package versions, or between a local package and a published version.

```
akuapkg diff <a> <b> [flags]
akuapkg diff <ref>                    # diff local HEAD against published ref
```

### Flags

| flag | description |
|---|---|
| `--format=<structural\|yaml\|both>` | diff level (default: structural) |
| `--scope=<schema\|sources\|manifests\|all>` | what to compare (default: all) |
| `--filter=<pattern>` | only show diffs matching pattern |

### Exit codes

0 if no structural changes, 1 if changes present. Useful for CI gates: non-zero = upgrade is breaking.

### JSON output

```json
{
  "schema": {
    "added": ["adminEmail"],
    "removed": [],
    "type_changed": [],
    "default_changed": [{"path": "replicas", "from": 3, "to": 5}]
  },
  "sources": {
    "added": [],
    "removed": [],
    "version_changed": [{"name": "cnpg", "from": "0.20.0", "to": "0.21.0"}]
  },
  "manifests": {
    "added": 2,
    "removed": 0,
    "modified": 4
  },
  "policy_compat": "allow"
}
```

---

## `akuapkg publish` ✅

Push a signed package to an OCI registry.

```
akuapkg publish [path] [flags]
```

### Flags

| flag | description |
|---|---|
| `--to=<oci-ref>` | destination (default: `[package].spec.publish.default`) |
| `--tag=<tag>` | tag (default: `[package].version`) |
| `--sign` | sign with configured cosign key (default: on if logged in) |
| `--attest` | emit and attach SLSA predicate (default: on) |
| `--public` | mark as public (required for ghcr public visibility) |

### Exit codes

0 success, 1 user error, 2 system error, 3 policy deny, 4 rate limited, 5 needs approval.

### JSON output

```json
{
  "package": "pkg.akua.dev/payments-api",
  "version": "3.2.0",
  "digest": "sha256:…",
  "signed": true,
  "attestation_digest": "sha256:…",
  "size_bytes": 1045832,
  "upload_duration_ms": 1823
}
```

---

## `akuapkg pull` ✅

Fetch a package from an OCI registry into the local cache.

```
akuapkg pull <ref> [flags]
```

### Flags

| flag | description |
|---|---|
| `--verify` | verify cosign signature (default: on) |
| `--unpack=<dir>` | unpack to directory instead of caching |
| `--insecure` | allow unsigned / unverifiable (dangerous) |

---

## `akuapkg inspect` ✅

Report a `package.k`'s input surface — every `option()` call-site with
its name, declared type, required flag, default, and help text.
Parse-only: the program is not executed.

```
akuapkg inspect [flags]
```

### Flags

| flag | description |
|---|---|
| `--package=<path>` | path to the `package.k` file (default `./package.k`) |

### Exit codes

0 success, 1 user error (missing file, parse failure), 2 system error.

### JSON output

```json
{
  "path": "./package.k",
  "options": [
    {
      "name": "input",
      "required": false
    }
  ]
}
```

Each option carries `name`, `required`, and optionally `type`,
`default`, `help` when the KCL source supplies them. `type` is
currently empty for the canonical `input: Input = ctx.input()` form —
kcl_lang's `list_options` only reads a type arg passed directly to
`option()`; full binding-context recovery arrives with AST walking.

> **SDK-first OCI inspection.** Published Akua Package inspection is
> available first through `@akua-dev/sdk` as `inspectOciPackage()`.
> The CLI target `akuapkg inspect <oci://...>` remains future work for
> full audit reports such as signatures, SLSA attestations, source
> provenance, and rendered-manifest counts.

---

## `akuapkg export` ✅

**Convert a Package's `Input` schema to a standard interchange format.** Emits JSON Schema 2020-12 (raw) or OpenAPI 3.1 (Input wrapped under `components.schemas`). Backed by KCL's resolver + AST walk; field docstrings become `description`, `@ui(...)` decorators become `x-ui` extensions.

```
akuapkg export --package <path> [--format=<json-schema|openapi>] [--out=<file>]
```

> **Not the same as `akuapkg render`.** `export` is format translation — it doesn't invoke Helm / kro / Kustomize and doesn't need customer inputs. It answers *"how do I describe this Package's inputs in a format other tools understand?"* Use `render` when you want deploy-ready manifests; use `export` when you want a schema for a UI form renderer or API doc generator. See [`akuapkg render`](#akuapkg-render) above.

### Supported formats

| format | output | for |
|---|---|---|
| `json-schema` (default) | JSON Schema Draft 2020-12 for the `Input` schema | install UIs, form renderers (rjsf, JSONForms) |
| `openapi` | OpenAPI 3.1 with `Input` under `components.schemas` | API docs (Swagger UI, Redoc), client SDK generation, admission-webhook validators |

### Flags

| flag | description |
|---|---|
| `--package=<path>` | path to `package.k` (default `./package.k`) |
| `--format=<fmt>` | `json-schema` (default) or `openapi` |
| `--out=<file>` | write to file (default: stdout) |

### `@ui(...)` decorators → `x-ui` extension

`@ui(...)` keyword arguments on schema attributes are projected onto the JSON Schema property as the OpenAPI-3.1-compliant `x-ui` extension. Renderers that recognise it (rjsf, custom form UIs) consume the hints; renderers that don't, ignore them.

```kcl
schema Input:
    @ui(order=10, group="Identity")
    name: str = "hello"

    @ui(order=20, widget="slider", min=1, max=20)
    replicas: int = 2
```

```json
{
  "properties": {
    "name": {
      "type": "string",
      "default": "hello",
      "x-ui": {"order": 10, "group": "Identity"}
    },
    "replicas": {
      "type": "integer",
      "default": 2,
      "x-ui": {"order": 20, "widget": "slider", "min": 1, "max": 20}
    }
  }
}
```

### Examples

```sh
# JSON Schema for a web form
akuapkg export --package package.k > inputs.schema.json

# OpenAPI 3.1 for API docs
akuapkg export --package package.k --format=openapi > package.openapi.json

# Write to file directly
akuapkg export --package package.k --out=exported/inputs.schema.json
```

### Exit codes

0 success; 1 if `package.k` lacks an `Input` schema or has KCL syntax errors; 5 on filesystem errors.

---

## `akuapkg api` ✅

Call the hosted Akua API from the OSS CLI. This is an optional hosted extension: local package workflows such as `render`, `export`, `check`, `lint`, `test`, and `verify` do not require hosted API credentials or network access.

```
akuapkg api <path-or-url> [flags]
akuapkg api spec [--audience=<public|partner|admin|internal>] [flags]
```

`<path-or-url>` can be a version-relative path such as `/workspaces` or an absolute URL on the configured API origin. Relative paths are resolved under the base URL. The default base URL is `https://api.akua.dev/v1/`.

### Examples

```sh
# List workspaces
akuapkg api /workspaces

# Create a product from a JSON body
akuapkg api /products -X POST --input product.json

# Send typed fields as JSON
akuapkg api /access_decisions -X POST -F permission=offers.create

# Send a workspace context header
akuapkg api /products --workspace ws_123

# Fetch the public OpenAPI document
akuapkg api spec

# Use a non-default API origin
akuapkg api /workspaces --base-url https://staging.example.dev/v1/
```

### Request flags

| flag | description |
|---|---|
| `-X, --method=<method>` | HTTP method. Defaults to `GET` when no body or fields are present, otherwise `POST` |
| `-H, --header=<name:value>` | extra request header. `name=value` is also accepted |
| `-f, --raw-field=<key=value>` | string field. Sent as query params for read methods and JSON body fields for writes without `--input`; when `--input` is present, fields stay in the query string |
| `-F, --field=<key=value>` | typed field. Parses `true`, `false`, `null`, and integers before sending |
| `--input=<file>` | JSON file to use as the request body |
| `--jq=<expr>` | reserved for response filtering; currently returns `E_UNSUPPORTED` |
| `--include` | reserved for response-header output; currently returns `E_UNSUPPORTED` |
| `--silent` | suppress a successful response body |
| `--paginate` | reserved for pagination; currently returns `E_UNSUPPORTED` |
| `--slurp` | reserved for paginated response aggregation; currently returns `E_UNSUPPORTED` |

### Connection flags

| flag | description |
|---|---|
| `--base-url=<url>` | hosted API base URL. Defaults to `https://api.akua.dev/v1/` |
| `--token=<token>` | bearer token for hosted API auth |
| `--workspace=<id>` | workspace context sent as the `akua-context` request header |

### Environment resolution

Connection values resolve in this order:

| setting | resolution |
|---|---|
| base URL | `--base-url`, then `AKUA_API_BASE_URL`, then `https://api.akua.dev/v1/` |
| bearer token | `--token`, then `AKUA_API_TOKEN` |
| workspace context | `--workspace`, then `AKUA_WORKSPACE_ID` |

`akuapkg api` uses hosted API bearer tokens only. `akuapkg auth` remains registry auth for OCI operations and is not reused for hosted API requests. A missing hosted API token fails with `E_AUTH_REQUIRED`; pass `--token` or set `AKUA_API_TOKEN`.

### `akuapkg api spec`

`akuapkg api spec` fetches the public OpenAPI document from `/openapi.json` on the configured base URL. `akuapkg api spec --audience public` is equivalent.

Elevated audiences are visible in the CLI contract but not served in this release:

```sh
akuapkg api spec --audience partner
akuapkg api spec --audience admin
akuapkg api spec --audience internal
```

Each elevated audience exits with `E_UNSUPPORTED` until the hosted API serves authorized audience-specific OpenAPI documents. The CLI does not locally filter the public OpenAPI document to simulate elevated audiences.

### Structured errors

Failed hosted API calls emit Akua structured errors on stderr. Under `--json` or agent context, stderr is JSON-lines:

```json
{"code":"E_AUTH_INVALID","message":"token is invalid or expired","docs":"https://cli.akua.dev/errors/E_AUTH_INVALID"}
```

HTTP `401` maps to auth errors, `403` maps to forbidden user errors, `429` exits with the rate-limited exit code, and transport/timeouts use the standard CLI contract exit codes.

### Exit codes

0 success, 1 user error, 2 system error, 4 rate limited, 6 timeout.

---

## `akuapkg dev` ✅

Start the hot-reload development loop.

```
akuapkg dev [flags]
```

Single long-running process. Watches workspace for changes. Renders, validates policy, applies to local target. Serves a browser UI at `http://localhost:5173`.

### Flags

| flag | description |
|---|---|
| `--target=<local\|dry-run\|cluster:<name>>` | apply target (default: local kind cluster) |
| `--port=<num>` | browser UI port (default: 5173) |
| `--policy=<tier>` | policy tier for live checks (default: `tier/dev`) |
| `--no-browser` | don't open browser automatically |
| `--fresh` | wipe persistent state before starting |
| `--inputs=<file>` | override inputs file |

### Exit codes

0 on clean shutdown (Ctrl-C), 1 for startup errors.

### JSON output (when `--json`)

Streaming JSON-lines of pipeline events:

```
{"t":1713636000,"stage":"render","app":"api","duration_ms":127,"status":"ok"}
{"t":1713636001,"stage":"policy","resource":"Deployment/api","verdict":"allow"}
{"t":1713636001,"stage":"apply","resource":"Deployment/api","op":"patch","duration_ms":198}
{"t":1713636002,"stage":"reconcile","resource":"Deployment/api","status":"ready"}
```

Useful for agents that want to drive `akuapkg dev` programmatically.

---

## `akuapkg whoami` ✅

Display current identity, logged-in registries, and scopes.

```
akuapkg whoami [flags]
```

### JSON output

```json
{
  "identity": "user@example.com",
  "registries": [
    {"url": "ghcr.io", "user": "robin", "expires_at": null},
    {"url": "akua.dev", "user": "robin", "tier": "team", "expires_at": "2026-05-20"}
  ],
  "scopes": ["packages:write", "policy:read"],
  "agent_context": {
    "detected": true,
    "agent": "claude-code",
    "source_env": "CLAUDECODE"
  }
}
```

`agent_context` is present when akua auto-detected an agent session (see [cli-contract.md §1.5](cli-contract.md#15-agent-context-auto-detection)). When no agent is detected, the field is `{"detected": false}`.

---

## `akuapkg test` ✅

Run unit tests for packages, policies, or both. Unified test runner across engines — detects target types by file extension.

```
akuapkg test [path] [flags]
```

Discovers and runs:

- `**/*_test.rego` — Rego policy tests via embedded OPA
- `**/*_test.k` / `test_*.k` — KCL test files via embedded KCL
- Kyverno `test.yaml` bundle tests (when the bundle is imported)
- Golden-output tests (`*.golden.yaml` compared against current render)

### Flags

| flag | description |
|---|---|
| `--coverage` | emit per-rule / per-schema coverage report |
| `--watch` | re-run on file change |
| `--golden` | enable / verify golden-output comparisons |
| `--filter=<regex>` | run only matching tests |
| `--timeout=<dur>` | per-test timeout (default 30s) |
| `--engine=<auto\|embedded\|shell>` | engine selection (see [embedded-engines.md](embedded-engines.md)) |

### Exit codes

0 if all pass, 1 if any fail, 2 on infrastructure error.

### JSON output

```json
{
  "summary": { "passed": 24, "failed": 1, "skipped": 2, "duration_ms": 413 },
  "results": [
    {
      "file":     "policies/production_test.rego",
      "test":     "test_deny_missing_team_label",
      "status":   "pass",
      "duration_ms": 12
    },
    {
      "file":     "packages/api/test_api.k",
      "test":     "test_default_replicas",
      "status":   "fail",
      "message":  "expected replicas=3, got 1",
      "duration_ms": 8
    }
  ],
  "coverage": { "overall": 0.72, "by_rule": { "deny_budget_exceeded": 0.0 } }
}
```

---

## `akuapkg fmt` ✅

Format KCL and Rego sources in place.

```
akuapkg fmt [path] [flags]
```

Uses embedded `kcl fmt` for `.k` files and embedded `opa fmt` for `.rego` files. Idempotent; safe to run in CI.

### Flags

| flag | description |
|---|---|
| `--check` | exit 1 if anything would change (CI gate); do not modify files |
| `--diff` | print unified diff of changes without applying |

### Exit codes

0 success, 1 formatting needed (with `--check`), 2 parse error.

---

## `akuapkg check` ✅

Syntax + type + dependency check. No execution, no rendering. Fast.

```
akuapkg check [path] [flags]
```

Stricter than `akuapkg lint` (actual compile errors, not style); cheaper than `akuapkg render` (doesn't invoke engines). Good for IDE save hooks and pre-commit.

### JSON output

```json
{
  "valid": true,
  "summary": { "files": 12, "errors": 0, "warnings": 0, "duration_ms": 89 }
}
```

On error:

```json
{
  "valid": false,
  "errors": [
    {
      "file":  "package.k",
      "line":  14,
      "code":  "E_SCHEMA_INVALID",
      "message": "expected int, got string",
      "suggestion": "remove quotes around value"
    }
  ]
}
```

---

## `akuapkg repl` ✅

Interactive REPL for exploring policies and packages.

```
akuapkg repl [flags]
```

Supports two modes (tab-switched):

- **Rego mode** — runs against the current policy set; evaluates expressions, shows trace, imports any `data.akua.policies.*`
- **KCL mode** — runs against the current package; evaluates expressions, shows schema types, hot-imports modules

Useful for experimenting before committing to a rule or package change.

---

## `akuapkg help` ✅

Print the top-level help or help for one command:

```sh
akuapkg help
akuapkg help render
```

---

## `akuapkg version` ✅

```
akuapkg version                 # print version + git SHA
akuapkg version --json
```

```json
{
  "version": "0.1.0",
  "commit": "abc123",
  "build_date": "2026-04-20",
  "go_version": "1.22",
  "rust_version": "1.82",
  "kcl_plugin_version": "0.1.0"
}
```

---

## Environment variables

A minimal set. No hidden state.

### akua-specific

| var | purpose |
|---|---|
| `AKUA_REGISTRY` | default OCI registry for publish/pull |
| `AKUA_CACHE_DIR` | override cache location (default: `$XDG_CACHE_HOME/akua`) |
| `AKUA_LOG_LEVEL` | override `--log-level` |
| `AKUA_NO_TELEMETRY` | force telemetry off (for CI) |
| `AKUA_TOKEN_FILE` | path to a token file for non-interactive auth |
| `AKUA_API_TOKEN` | hosted API bearer token for `akuapkg api` |
| `AKUA_API_BASE_URL` | hosted API base URL for `akuapkg api` (default: `https://api.akua.dev/v1/`) |
| `AKUA_WORKSPACE_ID` | workspace context sent by `akuapkg api` as `akua-context` |
| `AKUA_AGENT` | signal an agent context explicitly (value is the agent name) |
| `AKUA_NO_AGENT_DETECT` | disable agent-context auto-detection |

All of these can be overridden by flags where a flag exists. Local package workflows typically need none of them. Hosted API calls need a bearer token from `--token` or `AKUA_API_TOKEN`.

### Agent-context env vars (detected, never written)

These are set by agent runtimes, not by akua. akua reads them to determine whether it's running in an agent context.

| var | set by |
|---|---|
| `AGENT=<name>` | Goose (`goose`), Amp (`amp`), Codex (`codex`), Cline (`cline`), OpenCode (`opencode`) — emerging standard |
| `CLAUDECODE=1` | Claude Code |
| `GEMINI_CLI=1` | Gemini CLI |
| `CURSOR_CLI=1` | Cursor CLI |
| `GOOSE_TERMINAL=1`, `AMP_THREAD_ID=<id>`, `CODEX_SANDBOX=<id>`, `CLINE_ACTIVE=true` | secondary identifiers per agent — recorded as context |

See [cli-contract.md §1.5](cli-contract.md#15-agent-context-auto-detection) for detection rules and precedence.

---

## Exit code reference (summary)

From [cli-contract.md](cli-contract.md):

| code | meaning |
|---|---|
| 0 | success |
| 1 | user error |
| 2 | system error |
| 3 | policy deny |
| 4 | rate limited |
| 5 | needs approval |
| 6 | timeout |

---

## Stability and versioning

- Pre-v1.0: breaking changes require a minor version bump + changelog entry.
- v1.0 onward: flag removal requires 6-month deprecation; exit code semantics never change.
- JSON output keys are part of the stability contract.
- New verbs can be added without bumping major.

---

## What's not in this reference

- Implementation details (Rust crate structure, KCL plugin ABI).
- The TypeScript SDK (see [sdk.md](sdk.md)).
- The CLI contract (see [cli-contract.md](cli-contract.md)).
- Examples of usage (see [examples/](../examples/)).
- Architecture (see [architecture.md](architecture.md)).

## Spec cross-references

- **Package format** — [package-format.md](package-format.md) (KCL Package, four regions, engine callables)
- **Policy format** — [policy-format.md](policy-format.md) (Rego as host, compile-resolved imports, custom builtins)
- **Lockfile** — [lockfile-format.md](lockfile-format.md) (`akua.toml` + `akua.lock`)
