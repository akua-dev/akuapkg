# `E_AUTH_PARSE` — credential input is malformed

## What happened

A CLI `--auth` flag value, or the contents of an `--auth-file` TOML payload, didn't parse into a `(prefix, username, password)` triple. Akua rejects the input before it reaches the resolver — credentials that round-trip through a malformed parser are a class of bug we don't want to ship into the lockfile or HTTP transport.

This is distinct from [`E_INVALID_FLAG`](./E_INVALID_FLAG.md): that code covers structural CLI errors (`--timeout=5min`); `E_AUTH_PARSE` is specifically for credential-shape errors so agents can branch on it.

## Common causes

### `--auth` value missing the `=` separator

```sh
akuapkg vendor add upstream --auth github.com:alice:ghp_xyz   # no `=`
```

The expected shape is `<prefix>=<user>:<password>`. The split happens on the *first* `=` (so passwords containing `=` survive).

### `--auth` value missing the `:` separator inside credentials

```sh
akuapkg vendor add upstream --auth github.com=alice            # missing `:password`
```

The credential portion (right of the first `=`) splits on the *first* `:`.

### `--auth` value with an empty username or password

```sh
akuapkg vendor add upstream --auth github.com=:ghp_xyz         # empty username
akuapkg vendor add upstream --auth github.com=alice:           # empty password
```

Both halves of the credential must be non-empty.

### `--auth-file` points at a missing or unreadable path

```
--auth-file: ./missing.toml: No such file or directory
```

The path is treated as user-supplied input — akua does not fall back to a search path or default location.

### `--auth-file` TOML payload doesn't match the expected shape

```toml
# Wrong: top-level keys
"github.com/myco" = { username = "alice", password = "ghp_xyz" }

# Right: under [auth]
[auth]
"github.com/myco" = { username = "alice", password = "ghp_xyz" }
```

The file is a single TOML document with a `[auth]` table. Each entry's value is a table with exactly two string fields, `username` and `password`.

## How to fix it

### Inline flag

```sh
akuapkg vendor add upstream \
  --auth github.com/myco=alice:$GH_TOKEN \
  --auth gitlab.example.com=ci-bot:$GL_TOKEN
```

Repeat `--auth` for multiple hosts. Each value is one prefix-keyed credential.

### Auth file (TOML)

```toml
# auth.toml — explicit path, never auto-discovered
[auth]
"github.com/myco" = { username = "alice", password = "ghp_xyz..." }
"gitlab.example.com" = { username = "ci-bot", password = "glpat-..." }
```

```sh
akuapkg vendor add upstream --auth-file ./auth.toml
```

### Combining file and flag

Both are accepted on the same invocation. If a prefix appears in both, the flag value wins — same precedence as environment overrides over config files. This lets CI inject one-off overrides without rewriting the file:

```sh
akuapkg vendor add upstream \
  --auth-file ./auth.toml \
  --auth github.com/myco=alice:$ROTATED_TOKEN   # overrides the file entry
```

## Why akua doesn't auto-load `~/.netrc` / `~/.docker/config.json`

See [`E_MANIFEST_GIT_USERINFO`](./E_MANIFEST_GIT_USERINFO.md#why-no-netrc--dockerconfigjson-fallback) for the rationale. Short version: multi-tenant SDK consumers can't safely inherit ambient credentials, and the same explicit-input stance that keeps `akuapkg render` deterministic applies to credentials.

## Related

- [docs/cli.md → Auth flags](../cli.md#auth-flags-private-git-remotes) — full flag reference
- [docs/sdk.md → Credentials](../sdk.md#credentials---auth) — SDK equivalent
- [`E_MANIFEST_GIT_USERINFO`](./E_MANIFEST_GIT_USERINFO.md) — why credentials can't live in `akua.toml`
