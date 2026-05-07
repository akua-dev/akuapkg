#!/usr/bin/env bun
//! Render `site/errors/<CODE>.html` + `site/errors/index.html` from the
//! authoritative error-code list in `crates/akua-core/src/cli_contract/codes.rs`.
//!
//! For each code:
//!   - Parses the `pub const E_X: &str = "X";` declaration plus any
//!     immediately-preceding `///` rustdoc lines as a one-paragraph
//!     summary.
//!   - If `docs/errors/<CODE>.md` exists, renders that as the rich body
//!     (preferred for codes whose remediation is non-obvious — see
//!     E_PATH_ESCAPE for the model). Otherwise falls back to the rustdoc
//!     summary.
//!
//! Wired up via `task site:errors:gen`. The output under `site/errors/`
//! is committed; the deploy workflow (`.github/workflows/pages.yml`)
//! just copies it. Drift between codes.rs and the committed HTML is
//! the maintainer's problem at next regen — there's no CI guard.

import { readFileSync, writeFileSync, mkdirSync, existsSync, readdirSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

// ---------------------------------------------------------------------------
// Paths
// ---------------------------------------------------------------------------

const here = dirname(fileURLToPath(import.meta.url));
const root = join(here, '..');
const codesPath = join(root, 'crates/akua-core/src/cli_contract/codes.rs');
const richDocsDir = join(root, 'docs/errors');
const outDir = join(root, 'site/errors');

// ---------------------------------------------------------------------------
// Parse codes.rs
// ---------------------------------------------------------------------------

interface CodeEntry {
	/** `E_FOO_BAR` */
	name: string;
	/** Section header from the surrounding `// ----- Foo ---` comment. */
	section: string;
	/** Concatenated rustdoc above the const, paragraphs preserved. */
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
		// Anything else resets pendingDoc only if it's not a blank line — this lets
		// rustdoc paragraphs stay intact across single blank `///` lines.
		if (line.trim() !== '' && !line.startsWith('//')) {
			pendingDoc = [];
		}
	}
	return out;
}

// ---------------------------------------------------------------------------
// Tiny markdown → HTML renderer
//
// Covers the subset our error pages use: H1/H2/H3, paragraphs, fenced
// code blocks, inline code, bold, links, unordered + ordered lists.
// Intentionally minimal — no nested lists, no tables. If a future error
// page needs more, swap to `marked` (single-purpose dep).
// ---------------------------------------------------------------------------

function escape(s: string): string {
	return s
		.replace(/&/g, '&amp;')
		.replace(/</g, '&lt;')
		.replace(/>/g, '&gt;')
		.replace(/"/g, '&quot;');
}

/**
 * Relative `../foo.md` links in `docs/errors/*.md` resolve against the
 * `docs/` tree on GitHub (where the source markdown reads correctly),
 * but on the deployed site `akua.dev/errors/E_X` has no `../foo.md` to
 * point at — `docs/` isn't published. Rewrite those to absolute GitHub
 * URLs so the rendered page links land somewhere live.
 */
const GITHUB_BLOB = 'https://github.com/cnap-tech/akua/blob/main';

function rewriteUrl(url: string): string {
	if (url.startsWith('../') && url.endsWith('.md')) {
		return `${GITHUB_BLOB}/docs/${url.slice(3)}`;
	}
	if (url.startsWith('./') && url.endsWith('.md')) {
		return `${GITHUB_BLOB}/docs/errors/${url.slice(2)}`;
	}
	return url;
}

function renderInline(s: string): string {
	let out = escape(s);
	// inline code: `code`
	out = out.replace(/`([^`]+)`/g, (_, code) => `<code>${code}</code>`);
	// bold: **text**
	out = out.replace(/\*\*([^*]+)\*\*/g, '<strong>$1</strong>');
	// links: [text](url) — naive, doesn't handle nested brackets
	out = out.replace(
		/\[([^\]]+)\]\(([^)]+)\)/g,
		(_, text, url) => `<a href="${rewriteUrl(url)}">${text}</a>`,
	);
	return out;
}

function renderMarkdown(md: string): string {
	const lines = md.split('\n');
	const html: string[] = [];
	let i = 0;
	while (i < lines.length) {
		const line = lines[i];

		// Fenced code block.
		const fence = line.match(/^```\s*(\S*)\s*$/);
		if (fence) {
			const lang = fence[1];
			const buf: string[] = [];
			i++;
			while (i < lines.length && !/^```\s*$/.test(lines[i])) {
				buf.push(lines[i]);
				i++;
			}
			i++; // skip closing fence
			const cls = lang ? ` class="lang-${escape(lang)}"` : '';
			html.push(`<pre><code${cls}>${escape(buf.join('\n'))}</code></pre>`);
			continue;
		}

		// Heading.
		const heading = line.match(/^(#{1,3})\s+(.+)$/);
		if (heading) {
			const level = heading[1].length;
			html.push(`<h${level}>${renderInline(heading[2])}</h${level}>`);
			i++;
			continue;
		}

		// Unordered list.
		if (/^[-*]\s+/.test(line)) {
			const items: string[] = [];
			while (i < lines.length && /^[-*]\s+/.test(lines[i])) {
				items.push(`<li>${renderInline(lines[i].replace(/^[-*]\s+/, ''))}</li>`);
				i++;
			}
			html.push(`<ul>${items.join('')}</ul>`);
			continue;
		}

		// Ordered list.
		if (/^\d+\.\s+/.test(line)) {
			const items: string[] = [];
			while (i < lines.length && /^\d+\.\s+/.test(lines[i])) {
				items.push(`<li>${renderInline(lines[i].replace(/^\d+\.\s+/, ''))}</li>`);
				i++;
			}
			html.push(`<ol>${items.join('')}</ol>`);
			continue;
		}

		// Blockquote (single-line / paragraph).
		if (/^>\s*/.test(line)) {
			const buf: string[] = [];
			while (i < lines.length && /^>\s*/.test(lines[i])) {
				buf.push(lines[i].replace(/^>\s*/, ''));
				i++;
			}
			html.push(`<blockquote>${renderInline(buf.join(' '))}</blockquote>`);
			continue;
		}

		// Blank line — paragraph break.
		if (line.trim() === '') {
			i++;
			continue;
		}

		// Paragraph: gather until blank or block element.
		const buf: string[] = [line];
		i++;
		while (
			i < lines.length &&
			lines[i].trim() !== '' &&
			!/^(#{1,3}\s|```|[-*]\s|\d+\.\s|>\s)/.test(lines[i])
		) {
			buf.push(lines[i]);
			i++;
		}
		html.push(`<p>${renderInline(buf.join(' '))}</p>`);
	}
	return html.join('\n');
}

// ---------------------------------------------------------------------------
// Page templates
// ---------------------------------------------------------------------------

const STYLE = `
<style>
:root {
  color-scheme: dark;
  --bg: #0a0a0a;
  --fg: #e8e8ea;
  --muted: #8a8a8d;
  --line: #1b1b1d;
  --accent: #ee1c25;
  --code-bg: #111113;
}
* { box-sizing: border-box; }
html, body { margin: 0; padding: 0; }
body {
  background: var(--bg);
  color: var(--fg);
  font: 16px/1.6 ui-sans-serif, system-ui, -apple-system, "Segoe UI", Roboto, sans-serif;
  -webkit-font-smoothing: antialiased;
  min-height: 100vh;
  padding: 48px 20px;
}
.layout {
  display: grid;
  grid-template-columns: 240px minmax(0, 720px);
  gap: 56px;
  max-width: 1080px;
  margin: 0 auto;
  align-items: start;
}
.sidebar {
  position: sticky;
  top: 48px;
  max-height: calc(100vh - 96px);
  overflow-y: auto;
  font-size: 13px;
  padding-right: 8px;
}
.sidebar-title {
  margin: 0 0 16px;
  font-size: 11px;
  font-weight: 600;
  letter-spacing: 0.12em;
  text-transform: uppercase;
  color: var(--muted);
}
.sidebar h3 {
  margin: 18px 0 6px;
  font-size: 11px;
  font-weight: 600;
  letter-spacing: 0.08em;
  text-transform: uppercase;
  color: var(--muted);
}
.sidebar h3:first-of-type { margin-top: 0; }
.sidebar ul { list-style: none; padding: 0; margin: 0; }
.sidebar li { margin: 0; }
.sidebar a {
  display: block;
  padding: 4px 8px;
  margin: 1px -8px;
  font: 12px/1.4 ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
  color: var(--muted);
  text-decoration: none;
  border-radius: 4px;
  word-break: break-all;
}
.sidebar a:hover { color: var(--fg); background: var(--code-bg); }
.sidebar a.active {
  color: var(--fg);
  background: var(--code-bg);
  border-left: 2px solid var(--accent);
  padding-left: 6px;
}
main { width: 100%; max-width: 720px; min-width: 0; }
@media (max-width: 880px) {
  .layout { grid-template-columns: 1fr; gap: 0; }
  .sidebar { display: none; }
}
header { margin-bottom: 36px; }
header .crumbs {
  margin: 0 0 8px;
  color: var(--muted);
  font-size: 13px;
  letter-spacing: 0.02em;
}
header .crumbs a { color: var(--muted); text-decoration: underline; text-decoration-color: var(--line); text-underline-offset: 3px; }
header .crumbs a:hover { color: var(--fg); text-decoration-color: var(--accent); }
header h1 {
  margin: 0;
  font: 600 32px/1.2 ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
  letter-spacing: -0.01em;
}
header .tagline { margin: 6px 0 0; color: var(--muted); font-size: 16px; }
.section-tag {
  display: inline-block;
  margin: 0 0 16px;
  padding: 2px 10px;
  background: var(--code-bg);
  border: 1px solid var(--line);
  border-radius: 999px;
  color: var(--muted);
  font-size: 12px;
  letter-spacing: 0.06em;
  text-transform: uppercase;
}
h2 { margin: 32px 0 12px; font-size: 18px; font-weight: 600; }
h3 { margin: 24px 0 10px; font-size: 16px; font-weight: 600; }
p { margin: 0 0 14px; }
a { color: var(--fg); text-decoration: underline; text-decoration-color: var(--line); text-underline-offset: 3px; }
a:hover { text-decoration-color: var(--accent); }
code {
  font: 13px/1 ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
  background: var(--code-bg);
  border: 1px solid var(--line);
  border-radius: 4px;
  padding: 1px 5px;
}
pre {
  margin: 0 0 16px;
  padding: 14px 16px;
  background: var(--code-bg);
  border: 1px solid var(--line);
  border-radius: 6px;
  overflow-x: auto;
}
pre code {
  background: transparent;
  border: 0;
  padding: 0;
  font: 13px/1.55 ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
}
ul, ol { margin: 0 0 16px; padding-left: 24px; }
li { margin: 4px 0; }
blockquote {
  margin: 0 0 16px;
  padding: 8px 14px;
  border-left: 3px solid var(--accent);
  color: var(--muted);
  background: var(--code-bg);
  border-radius: 0 6px 6px 0;
}
footer { margin-top: 64px; padding-top: 16px; border-top: 1px solid var(--line); color: var(--muted); font-size: 13px; }
.code-list { list-style: none; padding: 0; margin: 0; }
.code-list li { margin: 0 0 14px; padding: 0; }
.code-list .name { font: 14px/1 ui-monospace, SFMono-Regular, Menlo, Consolas, monospace; }
.code-list .summary { color: var(--muted); font-size: 14px; margin-top: 4px; }
.section-group { margin-bottom: 32px; }
.section-group h2 { margin-bottom: 14px; }
</style>`.trim();

function pageShell(
	title: string,
	description: string,
	body: string,
	sidebar: string,
): string {
	return `<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8" />
<meta name="viewport" content="width=device-width, initial-scale=1" />
<title>${escape(title)} — akua</title>
<meta name="description" content="${escape(description)}" />
<meta name="color-scheme" content="dark" />
<meta property="og:title" content="${escape(title)} — akua" />
<meta property="og:description" content="${escape(description)}" />
<meta property="og:url" content="https://akua.dev/errors/" />
${STYLE}
</head>
<body>
<div class="layout">
${sidebar}
<main>
${body}
<footer>
  <a href="/">akua.dev</a> ·
  <a href="/errors/">All error codes</a> ·
  <a href="https://github.com/cnap-tech/akua">github.com/cnap-tech/akua</a>
</footer>
</main>
</div>
</body>
</html>
`;
}

/**
 * Same sidebar HTML on every page, except for the active-link highlight.
 * Pass `currentCode = null` for the index page (nothing highlighted).
 */
function renderSidebar(entries: CodeEntry[], currentCode: string | null): string {
	const grouped = new Map<string, CodeEntry[]>();
	for (const e of entries) {
		const arr = grouped.get(e.section) ?? [];
		arr.push(e);
		grouped.set(e.section, arr);
	}
	const sections = Array.from(grouped.entries())
		.map(([section, items]) => {
			const li = items
				.map((e) => {
					const cls = e.name === currentCode ? ' class="active"' : '';
					return `<li><a href="/errors/${escape(e.name)}"${cls}>${escape(e.name)}</a></li>`;
				})
				.join('');
			return `<h3>${escape(section)}</h3><ul>${li}</ul>`;
		})
		.join('\n');
	return `<aside class="sidebar">
<p class="sidebar-title"><a href="/errors/">All error codes</a></p>
${sections}
</aside>`;
}

function renderCodePage(
	entry: CodeEntry,
	richMarkdown: string | null,
	sidebar: string,
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
	return pageShell(entry.name, description, inner, sidebar);
}

function stripTags(html: string): string {
	return html.replace(/<[^>]+>/g, '').replace(/\s+/g, ' ');
}

function renderIndexPage(entries: CodeEntry[], sidebar: string): string {
	const grouped = new Map<string, CodeEntry[]>();
	for (const e of entries) {
		const arr = grouped.get(e.section) ?? [];
		arr.push(e);
		grouped.set(e.section, arr);
	}

	const sectionOrder = Array.from(grouped.keys());
	const groupsHtml = sectionOrder
		.map((section) => {
			const items = grouped.get(section)!;
			const li = items
				.map((e) => {
					const summary = e.summary
						? stripTags(renderMarkdown(e.summary)).slice(0, 220)
						: '';
					return `<li>
  <a class="name" href="./${escape(e.name)}">${escape(e.name)}</a>
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
	return pageShell(
		'Error codes',
		'Reference for every structured error code emitted by the akua CLI.',
		inner,
		sidebar,
	);
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

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
	const sidebar = renderSidebar(entries, entry.name);
	const html = renderCodePage(entry, richMd, sidebar);
	writeFileSync(join(outDir, `${entry.name}.html`), html);
	written++;
}

writeFileSync(
	join(outDir, 'index.html'),
	renderIndexPage(entries, renderSidebar(entries, null)),
);
console.log(`wrote ${written} code pages + index.html → site/errors/`);
