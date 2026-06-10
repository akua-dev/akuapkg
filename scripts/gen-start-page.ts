#!/usr/bin/env bun
//! Hand-authored getting-started page at `/start/`. Five-minute path
//! from install to first rendered Package, threaded through the
//! shared shell.

import { writeFileSync, mkdirSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { pageShell } from './site/layout.ts';
import { renderMarkdown } from './site/markdown.ts';

const here = dirname(fileURLToPath(import.meta.url));
const root = join(here, '..');
const outDir = join(root, 'site/start');

const md = `
> Pre-alpha. The path below works end-to-end on macOS / Linux. If you hit a wall, the [error code reference](/errors/) and the verb's [CLI page](/cli/) are the next stops.

## 1. Install

\`\`\`
$ curl -fsSL https://cli.akua.dev/install | sh
\`\`\`

Or grab a pinned binary from [Releases](https://github.com/cnap-tech/akua/releases). Windows: \`irm https://cli.akua.dev/install.ps1 | iex\`.

Verify the install:

\`\`\`
$ akua version
akua 0.8.7
\`\`\`

## 2. Initialize a workspace

\`\`\`
$ mkdir hello-akua && cd hello-akua
$ akua init
\`\`\`

This drops three files in your workspace:

- \`akua.toml\` — manifest. Declares the package metadata + dependencies.
- \`package.k\` — your Package, written in [KCL](https://www.kcl-lang.io/). One file, three regions: imports, schemas, body.
- \`inputs.example.yaml\` — example input values \`akua render\` reads when no \`--inputs\` is passed.

## 3. Render

\`\`\`
$ akua render
\`\`\`

By default, output goes to \`./rendered/\`. Each top-level Kubernetes resource your Package emits becomes its own YAML file, prefixed with a stable index so the layout is deterministic. Same inputs + same lockfile → byte-identical output, every time.

\`\`\`
rendered/
├── 000-deployment-app.yaml
├── 001-service-app.yaml
└── 002-configmap-config.yaml
\`\`\`

If anything breaks, akua emits a structured error on stderr with a stable \`code\` field and a \`docs\` URL pointing somewhere on this site:

\`\`\`json
{
  "level": "error",
  "code": "E_RENDER_KCL",
  "message": "schema validation failed: …",
  "docs": "https://akua.dev/errors/E_RENDER_KCL"
}
\`\`\`

Branch on \`code\` from agent code; the \`docs\` URL is the human-friendly fallback.

## 4. Add a dependency

Composing with an upstream Helm chart? \`akua add\` updates \`akua.toml\` and \`akua.lock\` in one step:

\`\`\`
$ akua add nginx --oci oci://ghcr.io/nginxinc/charts/nginx --version 1.0.0
\`\`\`

Then in \`package.k\`:

\`\`\`kcl
import charts.nginx as nginx
\`\`\`

The resolver writes a \`charts/<alias>/\` mount so the engine plugin (helm.template, kustomize.build, …) gets a path it produced itself — no path strings in your KCL.

## 5. Verify + sign + publish

When you're ready to ship:

\`\`\`
$ akua verify         # akua.toml ↔ akua.lock integrity + cosign signatures
$ akua sign           # cosign-sign the artifact
$ akua publish        # push the signed OCI artifact + SLSA attestation
\`\`\`

By default \`publish\` refuses unless the lockfile is clean and a cosign key is configured. See [Concepts → Security model](/concepts/security-model) for the threat model and what \`strict_signing\` enforces.

## What's next

- Walk through the runnable [examples](/examples/) — every one has \`package.k\`, \`inputs\`, and the rendered output side-by-side.
- Read [Concepts → Package format](/concepts/package-format) for the authoring shape.
- Skim the [CLI reference](/cli/) to see every verb at a glance — twenty-seven shipped, more in flight.
- The [SDK](/concepts/sdk) wraps the same surface for TypeScript / Bun / Deno consumers.

`.trim();

const body = `
<header>
  <h1>Get started</h1>
  <p class="tagline">Install akua, render your first Package, and ship a signed artifact — in five minutes.</p>
</header>
${renderMarkdown(md)}
`;

const html = pageShell({
	title: 'Get started',
	description:
		'Install akua, init a workspace, render your first Package, and publish a signed artifact.',
	body,
	currentSection: '/start/',
	sidebar: null,
	canonicalUrl: 'https://akua.dev/start/',
});

mkdirSync(outDir, { recursive: true });
writeFileSync(join(outDir, 'index.html'), html);
console.log('wrote site/start/index.html');
