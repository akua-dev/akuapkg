# `E_MANIFEST_GIT_USERINFO` — git URL embeds credentials

## What happened

A `git = "..."` dependency in `akua.toml` declares its URL with embedded basic-auth credentials — the URL's authority contains a `user[:pass]@` segment. Example:

```toml
[dependencies]
upstream = { git = "https://alice:ghp_xyz@github.com/myco/private", tag = "v1" }
```

Akua rejects this at parse time, before the dependency reaches the resolver or the lockfile.

Typical message:

```
dependency `upstream`: git URL must not embed credentials (`user:pass@`).
Pass credentials via the SDK `auth` parameter or the CLI `--auth` flag.
```

## Why akua refuses

`akua.toml` lives in version control. So does `akua.lock`, which records the canonical source URL of every resolved dep. If the manifest URL contained credentials, those credentials would be persisted into the lockfile, the git history of the project, and any artifact akua publishes — for the lifetime of the repository. Even rotating the token after the fact doesn't fully undo the leak (commit history, mirror clones, attacker forks).

Akua's rule: **credentials never appear in any file akua writes or reads as input.** The same principle that bans secrets in `kubectl apply -f`'d manifests applies to akua's dependency declarations.

The lockfile field `source` (see [lockfile-format.md](../lockfile-format.md)) canonicalizes URLs by stripping userinfo, default ports, and `.git` suffix — so even if a malformed call ever reached the lockfile-write path, the credential would not survive. This validation is the first line of defense; the canonicalization is the second.

## How to fix it

Pass credentials at the call site, never in `akua.toml`.

### From the SDK

```ts
await akua.vendorAdd('upstream', {
  auth: {
    'github.com/myco/private': { username: 'alice', password: process.env.GH_TOKEN! }
  }
});
```

The `auth` map is keyed by URL prefix (longest-prefix wins, same rule git's credential helper / `.npmrc` URL keys use). See [sdk.md → Credentials](../sdk.md#credentials---auth) for the full resolution rules.

### From the CLI

```sh
akuapkg vendor add upstream --auth github.com/myco/private=alice:$GH_TOKEN
```

Repeat `--auth` for multiple hosts. For environments where flags are awkward (CI, scripts), pass `--auth-file <path>` pointing at a TOML file you explicitly named:

```toml
# auth.toml
[auth]
"github.com/myco/private" = { username = "alice", password = "ghp_xyz..." }
```

```sh
akuapkg vendor add upstream --auth-file ./auth.toml
```

### Update your `akua.toml`

Strip the userinfo from the URL itself:

```toml
[dependencies]
# Before:
upstream = { git = "https://alice:ghp_xyz@github.com/myco/private", tag = "v1" }

# After:
upstream = { git = "https://github.com/myco/private", tag = "v1" }
```

Then re-run `akuapkg vendor add upstream` with the credential supplied via flag / SDK parameter.

## Why no `~/.netrc` / `~/.docker/config.json` fallback?

Akua deliberately does not auto-load ambient credential files. Two reasons:

- **Multi-tenant SDK consumers.** A server-side process embedding `@akua-dev/sdk` may handle requests from multiple tenants. Credentials that "happen to be on disk" can cross-contaminate between tenants. Explicit-only auth means each call carries exactly the credentials authorized for that call's principal.
- **Sandbox parity.** `akuapkg render` runs Packages in a wasmtime sandbox with strict capability scoping. Extending the same "no implicit input" stance to credentials is a coherent invariant — the SDK and CLI surface are the only places credentials enter the system.

## Related

- [docs/sdk.md → Credentials](../sdk.md#credentials---auth) — full SDK auth surface
- [docs/cli.md → akuapkg vendor → Auth flags](../cli.md#auth-flags-private-git-remotes) — CLI flag surface
- [docs/security-model.md](../security-model.md) — broader explicit-input stance
