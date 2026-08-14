# Using Akuapkg with AI agents

Akuapkg is the package-authoring tool. Agents can invoke it through the standalone `akuapkg` binary or through the Akua platform CLI as `akua pkg`. This page covers package workflows only; platform commands outside `akua pkg` belong to the separate `akua` CLI.

---

## The short version

- Akuapkg auto-detects agent sessions from standard environment variables (`AGENT=…`, `CLAUDECODE=1`, and others) and enables JSON output, structured errors, non-interactive mode, and no color. See [cli-contract.md §1.5](cli-contract.md#15-agent-context-auto-detection).
- The package command surface follows a strict contract: JSON output, typed exit codes, idempotent writes, plan mode, and time bounds. See [cli-contract.md](cli-contract.md).
- Agent-ready workflows ship as skills in [`skills/`](../skills/) following the open [Agent Skills Specification](https://agentskills.io).
- Akuapkg does not ship an MCP server. Agents use the CLI's structured output and load task-specific skills as needed.

---

## Why no MCP server?

Akuapkg exposes its complete package workflow through commands that work in a shell and return structured output. Repository skills add task guidance without duplicating the command surface in a second protocol:

1. **CLI surface with JSON-first output** (structured data for the agent; legible for humans).
2. **Skills in the repository** (natural-language task descriptions the agent loads on demand).
3. **Auto-detection of agent context** so the agent never has to remember `--json` or similar agent-specific flags.

This combination keeps package commands usable by both agents and people. Embedders can expose the same parser under a parent command without changing the contract.

For example, an agent can render directly or through the platform CLI:

```sh
akuapkg render --inputs inputs.yaml --out ./rendered
akua pkg render --inputs inputs.yaml --out ./rendered
```

---

## How Akuapkg auto-detects agent context

At process start, Akuapkg checks environment variables in this order:

| env var | agent |
|---|---|
| `AGENT=<name>` | Goose, Amp, Codex, Cline, OpenCode (standard) |
| `CLAUDECODE=1` | Claude Code |
| `GEMINI_CLI=1` | Gemini CLI |
| `CURSOR_CLI=1` | Cursor CLI |
| `AKUA_AGENT=<name>` | Akuapkg-specific fallback |

If any matches, Akuapkg silently enables `--json`, `--log=json`, `--no-color`, `--no-progress`, and `--no-interactive`. Explicit flags always win; use `--no-json` to force text output.

No stderr announcement. No prelude on stdout. Detection is observable via `akuapkg whoami --json` (reveals the `agent_context` field) or at `--log-level=debug`. Otherwise invisible.

---

## Load Akuapkg skills

The repository stores skills under [`skills/`](../skills/). Agents that discover repository-local Agent Skills can load them directly from a checkout. For global installation, follow the current instructions for your agent rather than assuming one command works across every client.

---

## Shipped skills

Nine skills cover common Akuapkg workflows. See [`skills/`](../skills/) for details.

| skill | use when |
|---|---|
| [new-package](../skills/new-package/) | user wants to start a new akua Package |
| [inspect-package](../skills/inspect-package/) | auditing a third-party Package before use |
| [diff-gate](../skills/diff-gate/) | setting up CI to block breaking upgrades |
| [dev-loop](../skills/dev-loop/) | iterating on a Package with hot-reload |
| [migrate-helmfile](../skills/migrate-helmfile/) | converting Helmfile to akua |
| [rotate-secret](../skills/rotate-secret/) | rotating a shared secret across installs |
| [publish-signed](../skills/publish-signed/) | releasing a signed + attested Package |
| [apply-policy-tier](../skills/apply-policy-tier/) | subscribing to a compliance / production tier |
| [test-and-lint](../skills/test-and-lint/) | checking package source and tests before review |

---

## Writing your own skill

Follow [the spec](https://agentskills.io/specification):

```
skills/my-skill/
├── SKILL.md              # required
├── scripts/              # optional — helper scripts
├── references/           # optional — long-form docs
└── assets/               # optional — templates, diagrams
```

`SKILL.md` minimum:

```markdown
---
name: my-skill
description: What this does and when to use it.
---

# My skill

Step-by-step instructions...
```

Validation: `npx skills-ref validate ./skills/my-skill`

Good descriptions include trigger keywords agents would recognize. Agents load the full body when they decide a skill applies.

See the [shipped skills](../skills/) for canonical examples.

---

## Running agents against Akuapkg — example loop

```
agent receives user intent:
  "add a Redis to my checkout service"

agent loads skills metadata:
  selects new-package + inspect-package

agent loads new-package SKILL.md fully:
  now has full procedure for scaffolding + adding sources

agent executes:
  akuapkg add redis --oci oci://registry-1.docker.io/bitnamicharts/redis --version 21.0.0
  edit package.k to wire redis values to existing schema
  akuapkg lint
  akuapkg render --inputs inputs.yaml --out ./rendered

agent verifies:
  akuapkg check
  akuapkg test

agent commits + opens PR:
  git commit -am "feat: add redis to checkout"
  gh pr create

CI runs:
  repository package checks

human reviews + approves + merges

deploy repo auto-updates; ArgoCD syncs
```

The workflow uses the package CLI, Git, and Markdown skills. The agent loads only the skill needed for the task.

---

## Related reading

- [cli-contract.md](cli-contract.md) — the CLI invariants agents rely on
- [cli.md](cli.md) — full verb reference
- [sdk.md](sdk.md) — programmatic access if you prefer SDK over CLI
- [agentskills.io](https://agentskills.io) — the skill-format standard
- [Cloudflare: code execution > MCP tools for agent ops](https://blog.cloudflare.com/code-mode-the-better-way-to-use-mcp/) — the research that shifted the ecosystem
