#!/usr/bin/env bun
//! Render `site/errors/<CODE>.html` + `site/errors/index.html` from
//! `crates/akua-core/src/cli_contract/codes.rs`.
//!
//! - Parses `pub const E_X: &str = "X";` plus its preceding `///`
//!   rustdoc as a one-paragraph summary.
//! - If `docs/errors/<CODE>.md` exists, renders it as the body
//!   (preferred for codes whose remediation is non-obvious — see
//!   E_PATH_ESCAPE for the model). Otherwise falls back to the
//!   rustdoc summary.
//!
//! Wired via `task site:errors:gen`. Also runs in `pages.yml` at
//! deploy time so the live site can never drift from codes.rs.

import { readFileSync, writeFileSync, mkdirSync, existsSync, readdirSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { escape, pageShell, stripTags, type SidebarSpec } from './site/layout.ts';
import { renderMarkdown } from './site/markdown.ts';

const here = dirname(fileURLToPath(import.meta.url));
const root = join(here, '..');
const codesPath = join(root, 'crates/akua-core/src/cli_contract/codes.rs');
const richDocsDir = join(root, 'docs/errors');
const outDir = join(root, 'site/errors');

interface CodeEntry {
	name: string;
	section: string;
	summary: string;
}

function parseCodes(src: string): CodeEntry[] {
	const lines = src.split('\n');
	const out: CodeEntry[] = [];
	let pendingDoc: string[] = [];
	let section = 'General';
	for (const raw of lines) {
		const line = raw.trimEnd();
		const sectionMatch = line.match(/^\/\/\s*-{3,}\s*(.+?)\s*-{3,}\s*$/);
		if (sectionMatch) {
			section = sectionMatch[1].trim();
			pendingDoc = [];
			continue;
		}
		const docMatch = line.match(/^\s*\/\/\/\s?(.*)$/);
		if (docMatch) {
			pendingDoc.push(docMatch[1]);
			continue;
		}
		const constMatch = line.match(/^pub const (E_[A-Z0-9_]+):\s*&str\s*=\s*"[^"]+";\s*$/);
		if (constMatch) {
			out.push({
				name: constMatch[1],
				section,
				summary: pendingDoc.join('\n').trim(),
			});
			pendingDoc = [];
			continue;
		}
		if (line.trim() !== '' && !line.startsWith('//')) {
			pendingDoc = [];
		}
	}
	return out;
}

function buildSidebar(entries: CodeEntry[], currentCode: string | null): SidebarSpec {
	const grouped = new Map<string, CodeEntry[]>();
	for (const e of entries) {
		const arr = grouped.get(e.section) ?? [];
		arr.push(e);
		grouped.set(e.section, arr);
	}
	return {
		rootHref: '/errors/',
		rootLabel: 'All error codes',
		sections: Array.from(grouped.entries()).map(([title, items]) => ({
			title,
			items: items.map((e) => ({
				href: `/errors/${e.name}`,
				label: e.name,
				active: e.name === currentCode,
			})),
		})),
	};
}

function renderCodePage(
	entry: CodeEntry,
	richMarkdown: string | null,
	sidebar: SidebarSpec,
): string {
	const summary = entry.summary || `Error code emitted by the akua CLI: ${entry.name}.`;
	const summaryHtml = renderMarkdown(summary);

	const bodyHtml = richMarkdown
		? renderMarkdown(richMarkdown)
		: `<h2>What happened</h2>${summaryHtml}<h2>How to fix it</h2><p>This error doesn't have an extended remediation guide yet — track <a href="https://github.com/cnap-tech/akua/issues">issues on GitHub</a> or open one with your <code>--json</code> output if the message above wasn't enough.</p>`;

	const inner = `
<header>
  <p class="crumbs"><a href="/">akua</a> / <a href="/errors/">errors</a> / ${escape(entry.name)}</p>
  <h1>${escape(entry.name)}</h1>
</header>
<p class="section-tag">${escape(entry.section)}</p>
${bodyHtml}
`;
	const description = stripTags(summaryHtml).slice(0, 200).trim();
	return pageShell({
		title: entry.name,
		description,
		body: inner,
		currentSection: '/errors/',
		sidebar,
		canonicalUrl: `https://akua.dev/errors/${entry.name}`,
	});
}

function renderIndexPage(entries: CodeEntry[], sidebar: SidebarSpec): string {
	const grouped = new Map<string, CodeEntry[]>();
	for (const e of entries) {
		const arr = grouped.get(e.section) ?? [];
		arr.push(e);
		grouped.set(e.section, arr);
	}

	const groupsHtml = Array.from(grouped.entries())
		.map(([section, items]) => {
			const li = items
				.map((e) => {
					const summary = e.summary
						? stripTags(renderMarkdown(e.summary)).slice(0, 220)
						: '';
					return `<li>
  <a class="name" href="/errors/${escape(e.name)}">${escape(e.name)}</a>
  ${summary ? `<div class="summary">${escape(summary)}</div>` : ''}
</li>`;
				})
				.join('\n');
			return `<section class="section-group"><h2>${escape(section)}</h2><ul class="code-list">${li}</ul></section>`;
		})
		.join('\n');

	const inner = `
<header>
  <p class="crumbs"><a href="/">akua</a> / errors</p>
  <h1>Error codes</h1>
  <p class="tagline">Every <code>akua</code> verb emits structured errors with a stable <code>code</code> field. Branch on these from agent code; humans get the full description by following the <code>docs</code> URL in the error JSON.</p>
</header>
${groupsHtml}
`;
	return pageShell({
		title: 'Error codes',
		description: 'Reference for every structured error code emitted by the akua CLI.',
		body: inner,
		currentSection: '/errors/',
		sidebar,
		canonicalUrl: 'https://akua.dev/errors/',
	});
}

const src = readFileSync(codesPath, 'utf8');
const entries = parseCodes(src);
console.log(`parsed ${entries.length} error codes from codes.rs`);

mkdirSync(outDir, { recursive: true });

const richDocsAvailable = existsSync(richDocsDir)
	? new Set(
			readdirSync(richDocsDir)
				.filter((f) => f.endsWith('.md'))
				.map((f) => f.replace(/\.md$/, '')),
		)
	: new Set<string>();

let written = 0;
for (const entry of entries) {
	const richMd = richDocsAvailable.has(entry.name)
		? readFileSync(join(richDocsDir, `${entry.name}.md`), 'utf8')
		: null;
	const html = renderCodePage(entry, richMd, buildSidebar(entries, entry.name));
	writeFileSync(join(outDir, `${entry.name}.html`), html);
	written++;
}

writeFileSync(join(outDir, 'index.html'), renderIndexPage(entries, buildSidebar(entries, null)));
console.log(`wrote ${written} code pages + index.html → site/errors/`);
