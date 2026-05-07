#!/usr/bin/env bun
//! Lift curated `docs/*.md` files onto the deployed site at `/concepts/<slug>`.
//!
//! Source markdown stays in `docs/` (so the GitHub view of the repo is
//! still useful); the generator just renders an HTML mirror with the
//! shared topnav + sidebar + style.

import { readFileSync, writeFileSync, mkdirSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { escape, pageShell, stripTags, type SidebarSpec } from './site/layout.ts';
import { renderMarkdown, type LinkResolverOpts } from './site/markdown.ts';

const here = dirname(fileURLToPath(import.meta.url));
const root = join(here, '..');
const docsDir = join(root, 'docs');
const outDir = join(root, 'site/concepts');

interface Concept {
	slug: string;
	title: string;
	mdFile: string;
	tagline: string;
}

// Curated allowlist — order matters for the sidebar. Kept small and
// stable; internal-process docs (impl-plan, design-notes, roadmap)
// stay GitHub-only.
const CONCEPTS: Concept[] = [
	{
		slug: 'package-format',
		mdFile: 'package-format.md',
		title: 'Package format',
		tagline: 'Package.k authoring shape — imports, schemas, body, and the `resources` output.',
	},
	{
		slug: 'lockfile-format',
		mdFile: 'lockfile-format.md',
		title: 'akua.toml + akua.lock',
		tagline: 'Manifest + lockfile shape, vendor-tree integration, and the verify pipeline.',
	},
	{
		slug: 'security-model',
		mdFile: 'security-model.md',
		title: 'Security model',
		tagline:
			'Wasmtime sandbox, capability-model preopens, replace-rejection in production, and the cosign + SLSA chain.',
	},
	{
		slug: 'embedded-engines',
		mdFile: 'embedded-engines.md',
		title: 'Embedded engines',
		tagline:
			'How KCL, Helm, OPA, Regal, Kustomize, kro, and the Kyverno→Rego converter ship as wasip1 modules inside akua.',
	},
	{
		slug: 'cli-contract',
		mdFile: 'cli-contract.md',
		title: 'CLI contract',
		tagline:
			'Universal verb invariants: `--json`, typed exit codes, structured errors, agent auto-detection.',
	},
	{
		slug: 'sdk',
		mdFile: 'sdk.md',
		title: 'TypeScript SDK',
		tagline:
			'Same surface as the CLI, exposed as `@akua-dev/sdk` with the helm + kustomize wasm engines bundled.',
	},
	{
		slug: 'agent-usage',
		mdFile: 'agent-usage.md',
		title: 'Agent usage',
		tagline:
			'Skill format, agent-context auto-detection, and the install paths for Claude Code / Cursor / Codex / Goose / 25+ others.',
	},
	{
		slug: 'debugging',
		mdFile: 'debugging.md',
		title: 'Debugging the render pipeline',
		tagline: 'Walking through a failing render with `--explain`, `RUST_LOG`, and `--dry-run`.',
	},
];

const SLUG_BY_NAME: Record<string, string> = Object.fromEntries(
	CONCEPTS.map((c) => [c.mdFile.replace(/\.md$/, ''), c.slug]),
);

/** Rewrite cross-doc links to their on-site path when the target is a
 *  concept we render; fall back to GitHub blob otherwise. */
function siteResolve(mdName: string): string | null {
	const slug = SLUG_BY_NAME[mdName];
	return slug ? `/concepts/${slug}` : null;
}

const linkOpts: LinkResolverOpts = { siteResolve };

function buildSidebar(currentSlug: string | null): SidebarSpec {
	return {
		rootHref: '/concepts/',
		rootLabel: 'Concepts',
		sections: [
			{
				items: CONCEPTS.map((c) => ({
					href: `/concepts/${c.slug}`,
					label: c.title,
					active: c.slug === currentSlug,
				})),
			},
		],
	};
}

function renderConceptPage(concept: Concept, md: string, sidebar: SidebarSpec): string {
	// Strip any leading top-level heading from the markdown — the page
	// header already shows the title. Avoids "Security model" appearing
	// twice on the page.
	const stripped = md.replace(/^#\s+.+\n+/, '').trim();
	const bodyHtml = renderMarkdown(stripped, linkOpts);

	const inner = `
<header>
  <p class="crumbs"><a href="/">akua</a> / <a href="/concepts/">concepts</a> / ${escape(concept.slug)}</p>
  <h1>${escape(concept.title)}</h1>
  <p class="tagline">${escape(concept.tagline)}</p>
</header>
${bodyHtml}
`;

	return pageShell({
		title: concept.title,
		description: concept.tagline,
		body: inner,
		currentSection: '/concepts/',
		sidebar,
		canonicalUrl: `https://akua.dev/concepts/${concept.slug}`,
	});
}

function renderIndexPage(sidebar: SidebarSpec): string {
	const items = CONCEPTS.map(
		(c) => `<li>
  <a class="name" href="/concepts/${escape(c.slug)}">${escape(c.title)}</a>
  <div class="summary">${escape(c.tagline)}</div>
</li>`,
	).join('\n');

	const inner = `
<header>
  <p class="crumbs"><a href="/">akua</a> / concepts</p>
  <h1>Concepts</h1>
  <p class="tagline">The "why is it like this?" docs. If you want to know how akua composes, what's enforced where, and what's deliberately not specified — start here.</p>
</header>
<ul class="code-list">${items}</ul>
`;

	return pageShell({
		title: 'Concepts',
		description: 'How akua works: package format, lockfile, sandbox, embedded engines, and the CLI contract.',
		body: inner,
		currentSection: '/concepts/',
		sidebar,
		canonicalUrl: 'https://akua.dev/concepts/',
	});
}

mkdirSync(outDir, { recursive: true });

let written = 0;
for (const concept of CONCEPTS) {
	const md = readFileSync(join(docsDir, concept.mdFile), 'utf8');
	const html = renderConceptPage(concept, md, buildSidebar(concept.slug));
	writeFileSync(join(outDir, `${concept.slug}.html`), html);
	written++;
}

writeFileSync(join(outDir, 'index.html'), renderIndexPage(buildSidebar(null)));
console.log(`wrote ${written} concept pages + index.html → site/concepts/`);
