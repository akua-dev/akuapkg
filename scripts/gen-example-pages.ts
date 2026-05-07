#!/usr/bin/env bun
//! Render the examples gallery at `/examples/` + per-example pages at
//! `/examples/<slug>`. Each example folder under `examples/<n>-<name>/`
//! contributes one page with its README, package.k, and rendered output
//! laid out side-by-side.

import { readFileSync, writeFileSync, mkdirSync, readdirSync, statSync, existsSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { escape, pageShell, stripTags, type SidebarSpec } from './site/layout.ts';
import { renderMarkdown, type LinkResolverOpts } from './site/markdown.ts';

const here = dirname(fileURLToPath(import.meta.url));
const root = join(here, '..');
const examplesDir = join(root, 'examples');
const outDir = join(root, 'site/examples');

interface Example {
	slug: string; // 01-hello-webapp
	dir: string; // absolute path
	title: string; // first H1 of README, fallback to slug
	tagline: string; // first paragraph of README
	readmeMd: string;
	packageK: string | null;
	rendered: { name: string; body: string }[]; // each rendered/*.yaml
}

function loadExamples(): Example[] {
	const entries = readdirSync(examplesDir)
		.filter((name) => /^\d{2}-/.test(name) && statSync(join(examplesDir, name)).isDirectory())
		.sort();
	return entries.map((slug) => {
		const dir = join(examplesDir, slug);
		const readmePath = join(dir, 'README.md');
		const readmeMd = existsSync(readmePath) ? readFileSync(readmePath, 'utf8') : '';
		const titleMatch = readmeMd.match(/^#\s+(.+)$/m);
		const title = titleMatch ? titleMatch[1].trim() : slug;
		const firstPara = readmeMd
			.split(/\n\s*\n/)
			.map((p) => p.trim())
			.find((p) => p && !p.startsWith('#') && !p.startsWith('>'));
		const tagline = firstPara ? stripTags(firstPara).slice(0, 200) : '';

		const packageKPath = join(dir, 'package.k');
		const packageK = existsSync(packageKPath) ? readFileSync(packageKPath, 'utf8') : null;

		const renderedDir = join(dir, 'rendered');
		const rendered = existsSync(renderedDir)
			? readdirSync(renderedDir)
					.filter((f) => f.endsWith('.yaml') || f.endsWith('.yml'))
					.sort()
					.map((f) => ({
						name: f,
						body: readFileSync(join(renderedDir, f), 'utf8'),
					}))
			: [];

		return { slug, dir, title, tagline, readmeMd, packageK, rendered };
	});
}

function buildSidebar(examples: Example[], currentSlug: string | null): SidebarSpec {
	return {
		rootHref: '/examples/',
		rootLabel: 'Examples',
		sections: [
			{
				items: examples.map((e) => ({
					href: `/examples/${e.slug}`,
					label: e.slug,
					active: e.slug === currentSlug,
				})),
			},
		],
	};
}

function renderExamplePage(example: Example, allSlugs: Set<string>, sidebar: SidebarSpec): string {
	const stripped = example.readmeMd.replace(/^#\s+.+\n+/, '').trim();
	const linkOpts: LinkResolverOpts = {
		sourceMd: `examples/${example.slug}/README.md`,
		repoResolve: (repoPath) => {
			// Sibling-example link `../<slug>/` → `/examples/<slug>`.
			const m = repoPath.match(/^examples\/([^/]+)\/?$/);
			if (m && allSlugs.has(m[1])) return `/examples/${m[1]}`;
			return null;
		},
	};
	const readmeHtml = renderMarkdown(stripped, linkOpts);

	const packageKBlock = example.packageK
		? `<h2>package.k</h2><pre><code class="lang-kcl">${escape(example.packageK)}</code></pre>`
		: '';

	const renderedBlock =
		example.rendered.length > 0
			? `<h2>Rendered output</h2>` +
				example.rendered
					.map(
						(r) =>
							`<h3>${escape(r.name)}</h3><pre><code class="lang-yaml">${escape(r.body)}</code></pre>`,
					)
					.join('\n')
			: '';

	const inner = `
<header>
  <p class="crumbs"><a href="/">akua</a> / <a href="/examples/">examples</a> / ${escape(example.slug)}</p>
  <h1>${escape(example.title)}</h1>
  ${example.tagline ? `<p class="tagline">${escape(example.tagline)}</p>` : ''}
</header>
${readmeHtml}
${packageKBlock}
${renderedBlock}
<p style="margin-top: 32px; color: var(--muted); font-size: 14px;">
  Source: <a href="https://github.com/cnap-tech/akua/tree/main/examples/${escape(example.slug)}">examples/${escape(example.slug)}/</a>
</p>
`;

	return pageShell({
		title: example.title,
		description: example.tagline || `akua example: ${example.slug}`,
		body: inner,
		currentSection: '/examples/',
		sidebar,
		canonicalUrl: `https://akua.dev/examples/${example.slug}`,
	});
}

function renderIndexPage(examples: Example[], sidebar: SidebarSpec): string {
	const items = examples
		.map(
			(e) => `<li>
  <a class="name" href="/examples/${escape(e.slug)}">${escape(e.slug)} — ${escape(e.title)}</a>
  ${e.tagline ? `<div class="summary">${escape(e.tagline)}</div>` : ''}
</li>`,
		)
		.join('\n');

	const inner = `
<header>
  <p class="crumbs"><a href="/">akua</a> / examples</p>
  <h1>Examples</h1>
  <p class="tagline">Runnable Packages, ordered from simplest to most realistic. Each one has a package.k, inputs, and a committed rendered/ output that integration tests diff against.</p>
</header>
<ul class="code-list">${items}</ul>
`;

	return pageShell({
		title: 'Examples',
		description: 'Runnable akua Packages — every one has rendered output committed.',
		body: inner,
		currentSection: '/examples/',
		sidebar,
		canonicalUrl: 'https://akua.dev/examples/',
	});
}

const examples = loadExamples();
console.log(`loaded ${examples.length} examples from examples/`);

mkdirSync(outDir, { recursive: true });

const allSlugs = new Set(examples.map((e) => e.slug));

let written = 0;
for (const example of examples) {
	const html = renderExamplePage(example, allSlugs, buildSidebar(examples, example.slug));
	writeFileSync(join(outDir, `${example.slug}.html`), html);
	written++;
}

writeFileSync(
	join(outDir, 'index.html'),
	renderIndexPage(examples, buildSidebar(examples, null)),
);
console.log(`wrote ${written} example pages + index.html → site/examples/`);
