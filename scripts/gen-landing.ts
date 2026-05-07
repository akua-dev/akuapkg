#!/usr/bin/env bun
//! Render `site/index.html` — the landing page. Hand-authored content,
//! threaded through the shared page shell so the topnav + logo + style
//! match the rest of the site.

import { writeFileSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { pageShell } from './site/layout.ts';

const here = dirname(fileURLToPath(import.meta.url));
const root = join(here, '..');

// Hero logo dwarfs the topnav badge so the landing reads as the
// "front door" of the brand. Smaller pages just get the topnav mark.
const body = `
<section style="text-align: center; padding: 24px 0 8px;">
  <img src="/assets/logo.png" alt="akua logo" style="width: 128px; height: 128px; margin: 0 auto 24px; display: block;" />
  <h1 style="font: 600 56px/1 ui-sans-serif, system-ui, sans-serif; letter-spacing: -0.03em; margin: 0 0 12px;">akua</h1>
  <p style="font-size: 19px; color: var(--muted); margin: 0 auto 40px; max-width: 560px;">
    One binary for cloud-native packaging.<br />
    Render, verify, sign, publish. KCL-typed. Helm and Kustomize embedded as WebAssembly. Sandboxed by default.
  </p>
</section>

<h2>Install (macOS / Linux)</h2>
<pre><code><span style="color: var(--accent); user-select: none;">$</span> curl -fsSL https://akua.dev/install | sh</code></pre>

<h2>Homebrew</h2>
<pre><code><span style="color: var(--accent); user-select: none;">$</span> brew install cnap-tech/tap/akua</code></pre>

<h2>Install (Windows)</h2>
<pre><code><span style="color: var(--accent); user-select: none;">&gt;</span> irm https://akua.dev/install.ps1 | iex</code></pre>

<h2>From source</h2>
<pre><code><span style="color: var(--accent); user-select: none;">$</span> cargo install --git https://github.com/cnap-tech/akua akua-cli</code></pre>

<h2>SDK (TypeScript / JavaScript)</h2>
<p style="color: var(--muted); font-size: 14px; margin-bottom: 12px;">
  Same render / verify / sign / publish surface as the CLI, exposed as a typed npm package
  with the helm + kustomize wasm engines bundled in. The native addon ships per-platform; no
  <code>akua</code> binary on <code>$PATH</code> required.
</p>
<pre><code><span style="color: var(--accent); user-select: none;">$</span> npm install @akua-dev/sdk</code></pre>
<pre><code><span style="color: var(--accent); user-select: none;">$</span> pnpm add @akua-dev/sdk</code></pre>
<pre><code><span style="color: var(--accent); user-select: none;">$</span> yarn add @akua-dev/sdk</code></pre>
<pre><code><span style="color: var(--accent); user-select: none;">$</span> bun add @akua-dev/sdk</code></pre>
<pre><code><span style="color: var(--accent); user-select: none;">$</span> deno add npm:@akua-dev/sdk</code></pre>

<h2>Documentation</h2>
<ul>
  <li><a href="/start/">Getting started</a> — install, init, render, publish in five minutes.</li>
  <li><a href="/cli/">CLI reference</a> — every verb, flag, and exit code.</li>
  <li><a href="/concepts/">Concepts</a> — sandboxing, determinism, signing, vendoring.</li>
  <li><a href="/examples/">Examples</a> — runnable Packages.</li>
  <li><a href="/errors/">Error codes</a> — the stable <code>code</code> field on every structured error.</li>
</ul>

<p style="color: var(--muted); font-size: 14px; margin-top: 24px;">
  Pre-alpha. Twenty-seven verbs wired behind one command surface. See the repo for what
  is and isn't done yet.
</p>
`.trim();

const html = pageShell({
	title: 'akua — one binary for cloud-native packaging',
	description:
		'One binary: render, verify, sign, publish. KCL-typed. Helm and Kustomize embedded as WebAssembly. Sandboxed by default.',
	body,
	currentSection: null,
	sidebar: null,
	canonicalUrl: 'https://akua.dev/',
});

writeFileSync(join(root, 'site/index.html'), html);
console.log('wrote site/index.html');
