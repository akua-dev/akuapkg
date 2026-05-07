#!/usr/bin/env bun
//! Split `docs/cli.md` into one HTML page per verb at `/cli/<verb>` plus
//! an index at `/cli/`.
//!
//! Each verb section in `cli.md` starts with `## \`akua <verb>\` <status>`
//! (status is ✅ shipped or 🚧 planned) and runs until the next `## ` or
//! the end of the file. The status emoji is preserved inline in the page
//! header, and the index lists shipped verbs separately from planned ones.

import { readFileSync, writeFileSync, mkdirSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { escape, pageShell, stripTags, type SidebarSpec } from './site/layout.ts';
import { renderMarkdown } from './site/markdown.ts';

const here = dirname(fileURLToPath(import.meta.url));
const root = join(here, '..');
const cliMdPath = join(root, 'docs/cli.md');
const outDir = join(root, 'site/cli');

interface Verb {
	name: string;
	status: 'shipped' | 'planned';
	body: string;
	tagline: string;
}

function parseVerbs(md: string): { intro: string; verbs: Verb[] } {
	const lines = md.split('\n');
	const verbs: Verb[] = [];
	const introLines: string[] = [];
	let current: { name: string; status: Verb['status']; lines: string[] } | null = null;

	for (const line of lines) {
		const header = line.match(/^##\s+`akua\s+(\S+)`\s*(✅|🚧)?/);
		if (header) {
			if (current) {
				verbs.push(finishVerb(current));
			}
			current = {
				name: header[1],
				status: header[2] === '🚧' ? 'planned' : 'shipped',
				lines: [],
			};
			continue;
		}
		// `---` between verbs in cli.md is a section divider — drop.
		if (line.trim() === '---' && current) continue;
		if (current) {
			current.lines.push(line);
		} else {
			introLines.push(line);
		}
	}
	if (current) verbs.push(finishVerb(current));

	return { intro: introLines.join('\n').trim(), verbs };
}

function finishVerb(c: { name: string; status: Verb['status']; lines: string[] }): Verb {
	const body = c.lines.join('\n').trim();
	// Tagline = first non-empty paragraph of the verb body.
	const firstPara = body.split(/\n\s*\n/)[0]?.trim() ?? '';
	const tagline = stripTags(firstPara).slice(0, 240);
	return {
		name: c.name,
		status: c.status,
		body,
		tagline,
	};
}

function buildSidebar(verbs: Verb[], currentVerb: string | null): SidebarSpec {
	const shipped = verbs.filter((v) => v.status === 'shipped');
	const planned = verbs.filter((v) => v.status === 'planned');
	const toItems = (vs: Verb[]) =>
		vs.map((v) => ({
			href: `/cli/${v.name}`,
			label: v.name,
			active: v.name === currentVerb,
		}));
	const sections: SidebarSpec['sections'] = [
		{ title: 'Shipped', items: toItems(shipped) },
	];
	if (planned.length > 0) {
		sections.push({ title: 'Planned', items: toItems(planned) });
	}
	return {
		rootHref: '/cli/',
		rootLabel: 'CLI reference',
		sections,
	};
}

function renderVerbPage(verb: Verb, sidebar: SidebarSpec): string {
	const status = verb.status === 'shipped' ? 'Shipped' : 'Planned';
	const inner = `
<header>
  <p class="crumbs"><a href="/">akua</a> / <a href="/cli/">cli</a> / ${escape(verb.name)}</p>
  <h1>akua ${escape(verb.name)}</h1>
</header>
<p class="section-tag">${status}</p>
${renderMarkdown(verb.body)}
`;
	return pageShell({
		title: `akua ${verb.name}`,
		description: verb.tagline,
		body: inner,
		currentSection: '/cli/',
		sidebar,
		canonicalUrl: `https://akua.dev/cli/${verb.name}`,
	});
}

function renderIndexPage(intro: string, verbs: Verb[], sidebar: SidebarSpec): string {
	const grouped = {
		Shipped: verbs.filter((v) => v.status === 'shipped'),
		Planned: verbs.filter((v) => v.status === 'planned'),
	};
	const groupHtml = (title: string, vs: Verb[]) => {
		if (vs.length === 0) return '';
		const li = vs
			.map(
				(v) => `<li>
  <a class="name" href="/cli/${escape(v.name)}">akua ${escape(v.name)}</a>
  <div class="summary">${escape(v.tagline)}</div>
</li>`,
			)
			.join('\n');
		return `<section class="section-group"><h2>${escape(title)} (${vs.length})</h2><ul class="code-list">${li}</ul></section>`;
	};

	const inner = `
<header>
  <p class="crumbs"><a href="/">akua</a> / cli</p>
  <h1>CLI reference</h1>
  <p class="tagline">Every <code>akua</code> verb. Shipped verbs are wired and tested; planned verbs have a stable surface but no backing implementation yet.</p>
</header>
${renderMarkdown(intro)}
${groupHtml('Shipped', grouped.Shipped)}
${groupHtml('Planned', grouped.Planned)}
`;
	return pageShell({
		title: 'CLI reference',
		description: 'Every akua verb, flag, and exit code.',
		body: inner,
		currentSection: '/cli/',
		sidebar,
		canonicalUrl: 'https://akua.dev/cli/',
	});
}

const md = readFileSync(cliMdPath, 'utf8');
const { intro, verbs } = parseVerbs(md);
console.log(`parsed ${verbs.length} verbs from cli.md`);

mkdirSync(outDir, { recursive: true });

let written = 0;
for (const verb of verbs) {
	const html = renderVerbPage(verb, buildSidebar(verbs, verb.name));
	writeFileSync(join(outDir, `${verb.name}.html`), html);
	written++;
}

writeFileSync(join(outDir, 'index.html'), renderIndexPage(intro, verbs, buildSidebar(verbs, null)));
console.log(`wrote ${written} verb pages + index.html → site/cli/`);
