//! Tiny markdown-to-HTML renderer covering the subset our site uses.
//!
//! Supports H1/H2/H3, paragraphs, fenced code blocks, inline code,
//! bold, italic, links, unordered + ordered lists, blockquotes, and
//! GFM tables (used by docs/cli.md). Intentionally minimal — no
//! nested lists, no syntax highlighting. If a future page needs more,
//! swap to `marked` (single-purpose dep).

import { escape } from './layout.ts';

/** Github-blob base for rewriting `../foo.md` links found in `docs/`. */
const GITHUB_BLOB = 'https://github.com/cnap-tech/akua/blob/main';

/**
 * Rewrite relative markdown links to absolute GitHub URLs. Source
 * markdown lives under `docs/` where `../foo.md` resolves correctly,
 * but the published site doesn't serve `docs/` so those links would
 * 404 without rewriting.
 *
 * Override `siteResolve` to redirect specific docs links into the
 * deployed-site equivalent (e.g. `package-format.md` → `/concepts/package-format`).
 */
export interface LinkResolverOpts {
	/** Map a `<name>.md` (without the `.md`) to a site URL, or return
	 *  null to fall back to the GitHub blob URL. */
	siteResolve?: (mdName: string) => string | null;
}

export function rewriteUrl(url: string, opts: LinkResolverOpts = {}): string {
	const mdMatch = url.match(/^(?:\.\.\/|\.\/|\/?docs\/)?(.+)\.md$/);
	if (mdMatch && !url.startsWith('http') && !url.startsWith('#')) {
		const name = mdMatch[1].replace(/^errors\//, '');
		if (opts.siteResolve) {
			const resolved = opts.siteResolve(name);
			if (resolved) return resolved;
		}
		// Fallback: link to the markdown source on GitHub.
		const path = url.startsWith('../')
			? `/docs/${url.slice(3)}`
			: url.startsWith('./')
				? `/docs/errors/${url.slice(2)}`
				: url.startsWith('/docs/')
					? url
					: `/docs/${url}`;
		return `${GITHUB_BLOB}${path}`;
	}
	return url;
}

function renderInline(s: string, opts: LinkResolverOpts): string {
	let out = escape(s);
	out = out.replace(/`([^`]+)`/g, (_, code) => `<code>${code}</code>`);
	out = out.replace(/\*\*([^*]+)\*\*/g, '<strong>$1</strong>');
	// italic — only single-asterisk, avoid clobbering ** above
	out = out.replace(/(?<![*\w])\*([^*\n]+)\*(?!\w)/g, '<em>$1</em>');
	out = out.replace(
		/\[([^\]]+)\]\(([^)]+)\)/g,
		(_, text, url) => `<a href="${rewriteUrl(url, opts)}">${text}</a>`,
	);
	return out;
}

/** Render a GFM-style table block. Lines is the slice between fences. */
function renderTable(lines: string[], opts: LinkResolverOpts): string {
	const cells = (line: string): string[] =>
		line
			.replace(/^\|/, '')
			.replace(/\|$/, '')
			.split('|')
			.map((c) => c.trim());
	const header = cells(lines[0]);
	// row 1 is the separator (---|---) — skip
	const rows = lines.slice(2).map(cells);
	const thead = `<thead><tr>${header.map((h) => `<th>${renderInline(h, opts)}</th>`).join('')}</tr></thead>`;
	const tbody = `<tbody>${rows
		.map(
			(row) =>
				`<tr>${row.map((c) => `<td>${renderInline(c, opts)}</td>`).join('')}</tr>`,
		)
		.join('')}</tbody>`;
	return `<table>${thead}${tbody}</table>`;
}

export function renderMarkdown(md: string, opts: LinkResolverOpts = {}): string {
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
			i++;
			const cls = lang ? ` class="lang-${escape(lang)}"` : '';
			html.push(`<pre><code${cls}>${escape(buf.join('\n'))}</code></pre>`);
			continue;
		}

		// Heading.
		const heading = line.match(/^(#{1,4})\s+(.+)$/);
		if (heading) {
			const level = heading[1].length;
			const text = heading[2].replace(/\s*\{[^}]*\}\s*$/, ''); // strip {#anchor}
			const slug = text
				.toLowerCase()
				.replace(/[^a-z0-9 -]/g, '')
				.trim()
				.replace(/\s+/g, '-');
			html.push(
				`<h${level} id="${escape(slug)}">${renderInline(text, opts)}</h${level}>`,
			);
			i++;
			continue;
		}

		// Horizontal rule.
		if (/^---+\s*$/.test(line)) {
			html.push('<hr />');
			i++;
			continue;
		}

		// Table — line starts with `|` and the next is a separator (---|---).
		if (line.startsWith('|') && i + 1 < lines.length && /^\|?[\s:|-]+\|?\s*$/.test(lines[i + 1])) {
			const buf: string[] = [];
			while (i < lines.length && lines[i].startsWith('|')) {
				buf.push(lines[i]);
				i++;
			}
			html.push(renderTable(buf, opts));
			continue;
		}

		// Unordered list.
		if (/^[-*]\s+/.test(line)) {
			const items: string[] = [];
			while (i < lines.length && /^[-*]\s+/.test(lines[i])) {
				items.push(`<li>${renderInline(lines[i].replace(/^[-*]\s+/, ''), opts)}</li>`);
				i++;
			}
			html.push(`<ul>${items.join('')}</ul>`);
			continue;
		}

		// Ordered list.
		if (/^\d+\.\s+/.test(line)) {
			const items: string[] = [];
			while (i < lines.length && /^\d+\.\s+/.test(lines[i])) {
				items.push(
					`<li>${renderInline(lines[i].replace(/^\d+\.\s+/, ''), opts)}</li>`,
				);
				i++;
			}
			html.push(`<ol>${items.join('')}</ol>`);
			continue;
		}

		// Blockquote.
		if (/^>\s*/.test(line)) {
			const buf: string[] = [];
			while (i < lines.length && /^>\s*/.test(lines[i])) {
				buf.push(lines[i].replace(/^>\s*/, ''));
				i++;
			}
			html.push(`<blockquote>${renderInline(buf.join(' '), opts)}</blockquote>`);
			continue;
		}

		// Blank line — paragraph break.
		if (line.trim() === '') {
			i++;
			continue;
		}

		// Paragraph.
		const buf: string[] = [line];
		i++;
		while (
			i < lines.length &&
			lines[i].trim() !== '' &&
			!/^(#{1,4}\s|```|[-*]\s|\d+\.\s|>\s|---+\s*$|\|)/.test(lines[i])
		) {
			buf.push(lines[i]);
			i++;
		}
		html.push(`<p>${renderInline(buf.join(' '), opts)}</p>`);
	}
	return html.join('\n');
}
