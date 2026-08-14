# Embedded engines

Akuapkg embeds its KCL runtime and the supported Helm and Kustomize engines in the standalone `akuapkg` binary. Package commands do not look up `kcl`, `helm`, or `kustomize` on `$PATH`.

The Akua platform CLI can expose the same package dispatcher under `akua pkg`. This page uses Akuapkg and `akuapkg` for the package tool unless it is describing that outer invocation.

This page explains how the engines are embedded and what the design means for package authors, agents, and CI. See [security-model.md](security-model.md) for the render sandbox threat model.

---

## Why embed

Akuapkg embeds engines for two reasons:

1. **Single binary workflow.** A published Akuapkg binary includes the supported package engines. You do not need separate KCL, Helm, or Kustomize installations.
2. **Version determinism.** An Akuapkg version uses the engine versions built with that release instead of whichever executables happen to be on `$PATH`.

Agents and CI jobs can therefore run package commands without installing engine-specific binaries.

---

## Embedding strategy

The default standalone build packages the render worker and engine modules with the `akuapkg` executable:

```text
engine source (Go or Rust)
        │
        ▼
compiled or linked into the render runtime
        │
        ▼
packaged with the akuapkg binary
        │
        ▼
hosted by a shared Wasmtime Engine
(one Engine per process; separate Stores per invocation)
```

Akuapkg uses one process-wide `engine_host_wasm::shared_engine()` instance. Each render worker and engine call receives a separate Wasmtime Store on that Engine.

Sandboxed KCL calls a single host import, `env::kcl_plugin_invoke_json_wasm`, to reach the registered `akua-core` plugin handler. The handler runs the requested engine, returns the result bytes, and resumes the render worker. See [the shared-engine architecture](security-model.md#one-engine-many-stores--with-a-plugin-bridge) for the boundary details.

Build scripts precompile the render worker and supported WASM engines against the shared Wasmtime configuration. The standalone binary embeds those artifacts by default.

---

## Engine inventory

The current standalone source enables these engines:

| engine | implementation | status |
|---|---|---|
| KCL | Rust runtime linked into the render worker | shipped |
| Helm | Go engine compiled to `wasip1` and hosted by Wasmtime | shipped |
| Kustomize | Go engine compiled to `wasip1` and hosted by Wasmtime | shipped |

Akuapkg does not fall back to shelling out when an engine is unavailable.

---

## Command routing

Current package commands use the embedded runtime as follows:

| command | runtime used |
|---|---|
| `akuapkg render` | KCL, plus Helm or Kustomize when the package calls those plugins |
| `akuapkg check` | KCL package checks |
| `akuapkg lint` | KCL parsing and import checks |
| `akuapkg fmt` | KCL formatter |
| `akuapkg test` | KCL test programs |
| `akuapkg repl` | KCL evaluator |

Run `akuapkg --help` for the current standalone command list.

---

## Determinism guarantees

Embedded engines are pinned to the Akuapkg build. Two `akuapkg render` runs with the same package source, inputs, lockfile, and Akuapkg version produce byte-identical output. See [the CLI determinism contract](cli-contract.md#13-determinism).

---

## Security posture

The render worker and embedded engine modules run inside Wasmtime stores with explicit resource and capability limits:

- Package code can access only explicitly preopened paths.
- Package code cannot make network requests.
- Package code cannot read host environment variables.
- Package code cannot start subprocesses.

Akuapkg does not include a shell-out fallback for the render pipeline. Package paths also pass through `akua-core` guards before an engine receives them.

---

## For agents

An agent can use `akuapkg render`, `akuapkg test`, and `akuapkg fmt` without checking for separate KCL, Helm, or Kustomize executables. The command's structured output and exit codes remain the agent-facing interface.

---

## Relationship to other docs

- [CLI reference](cli.md) lists the standalone package commands.
- [Package format](package-format.md) explains KCL package source.
- [Security model](security-model.md) defines the sandbox boundary.
- [CLI determinism contract](cli-contract.md#13-determinism) defines reproducibility.
