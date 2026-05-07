#!/usr/bin/env bun
//! Walk every generated `site/**/*.html`, extract every `href` and
//! `src`, and flag any internal link whose target doesn't exist on
//! disk. Exits non-zero on the first broken link so `task site:gen`
//! and the Pages workflow fail loudly when a regenerator drops a doc
//! we still link to.
//!
//! What counts as broken:
//!   /foo            → site/foo/index.html missing
//!   /foo.html       → site/foo.html missing
//!   /foo#bar        → site/foo/index.html exists, but no `id="bar"`
//!                     (anchor target missing)
//!   ./bar.md        → relative .md links shouldn't survive into HTML
//!                     (markdown.ts rewrites them); flag any that did
//!
//! What we skip:
//!   http(s)://…     → external; we don't fetch
//!   mailto: / tel:  → not navigable HTML
//!   #frag-only      → in-page anchor; checked against same file

import { readFileSync, readdirSync, statSync } from 'node:fs';
import { join, dirname, relative, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const root = join(here, '..');
const siteDir = join(root, 'site');

interface BrokenLink {
	file: string;
	url: string;
	reason: string;
}

function* walk(dir: string): Generator<string> {
	for (const entry of readdirSync(dir, { withFileTypes: true })) {
		const full = join(dir, entry.name);
		if (entry.isDirectory()) yield* walk(full);
		else if (entry.isFile() && entry.name.endsWith('.html')) yield full;
	}
}

function existsFile(p: string): boolean {
	try {
		return statSync(p).isFile();
	} catch {
		return false;
	}
}

/** Collect every `id="…"` value present in `html`. */
function collectAnchors(html: string): Set<string> {
	const ids = new Set<string>();
	for (const m of html.matchAll(/\sid="([^"]+)"/g)) ids.add(m[1]);
	return ids;
}

/**
 * Resolve a `/path` URL to the on-disk HTML file the server would
 * serve. Mirrors GitHub Pages' static behaviour:
 *   /foo       → site/foo.html     (if exists)  or  site/foo/index.html
 *   /foo/      → site/foo/index.html
 *   /foo.html  → site/foo.html
 */
function resolveSitePath(urlPath: string): string | null {
	const trimmed = urlPath.replace(/^\//, '');
	if (trimmed === '') return join(siteDir, 'index.html');

	const direct = join(siteDir, trimmed);
	if (existsFile(direct)) return direct;
	if (existsFile(direct + '.html')) return direct + '.html';
	if (existsFile(join(direct, 'index.html'))) return join(direct, 'index.html');
	return null;
}

function checkFile(absHtmlPath: string, html: string): BrokenLink[] {
	const broken: BrokenLink[] = [];
	const selfAnchors = collectAnchors(html);
	const selfRel = relative(root, absHtmlPath);

	// Match href="…" and src="…". Single-quoted attrs are not produced
	// by our generators, so this is sufficient.
	const attrRe = /(?:href|src)="([^"]+)"/g;
	for (const m of html.matchAll(attrRe)) {
		const url = m[1];

		// Skip external + non-navigable.
		if (/^(?:https?:|mailto:|tel:|data:)/i.test(url)) continue;
		// Skip empty / placeholder.
		if (url === '' || url === '#') continue;

		// Pure in-page anchor.
		if (url.startsWith('#')) {
			const id = url.slice(1);
			if (!selfAnchors.has(id)) {
				broken.push({ file: selfRel, url, reason: `no #${id} on this page` });
			}
			continue;
		}

		// Reject any `.md` link that survived into the HTML — the
		// markdown renderer should have rewritten it; if not, it's
		// almost certainly a dead link.
		if (/\.md(?:[#?]|$)/.test(url)) {
			broken.push({
				file: selfRel,
				url,
				reason: 'unrewritten .md link (markdown renderer should have caught this)',
			});
			continue;
		}

		// Split off fragment.
		const hashIdx = url.indexOf('#');
		const path = hashIdx >= 0 ? url.slice(0, hashIdx) : url;
		const frag = hashIdx >= 0 ? url.slice(hashIdx + 1) : '';

		// Resolve to absolute site path.
		let absPath: string | null;
		if (path.startsWith('/')) {
			absPath = resolveSitePath(path);
		} else {
			// Relative — resolve against the current file's dir,
			// then verify it lives under siteDir.
			const target = resolve(dirname(absHtmlPath), path);
			if (!target.startsWith(siteDir)) {
				broken.push({ file: selfRel, url, reason: 'relative link escapes site/' });
				continue;
			}
			if (existsFile(target)) absPath = target;
			else if (existsFile(join(target, 'index.html'))) absPath = join(target, 'index.html');
			else absPath = null;
		}

		if (!absPath) {
			broken.push({ file: selfRel, url, reason: 'target file not found' });
			continue;
		}

		// Anchor check — only when the path resolved to an HTML page.
		if (frag && absPath.endsWith('.html')) {
			const targetHtml = absPath === absHtmlPath ? html : readFileSync(absPath, 'utf8');
			const ids = absPath === absHtmlPath ? selfAnchors : collectAnchors(targetHtml);
			if (!ids.has(frag)) {
				broken.push({
					file: selfRel,
					url,
					reason: `target page has no #${frag}`,
				});
			}
		}
	}
	return broken;
}

const allBroken: BrokenLink[] = [];
let scanned = 0;
for (const file of walk(siteDir)) {
	scanned++;
	const html = readFileSync(file, 'utf8');
	allBroken.push(...checkFile(file, html));
}

if (allBroken.length === 0) {
	console.log(`link check: ${scanned} pages, 0 broken links`);
	process.exit(0);
}

// Group by file for readable output.
const byFile = new Map<string, BrokenLink[]>();
for (const b of allBroken) {
	const list = byFile.get(b.file) ?? [];
	list.push(b);
	byFile.set(b.file, list);
}

console.error(`link check: ${allBroken.length} broken link(s) across ${byFile.size} page(s):\n`);
for (const [file, links] of [...byFile.entries()].sort()) {
	console.error(`  ${file}`);
	for (const l of links) {
		console.error(`    → ${l.url}  (${l.reason})`);
	}
}
process.exit(1);
