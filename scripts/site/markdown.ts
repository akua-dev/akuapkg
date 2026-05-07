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
const GITHUB_TREE = 'https://github.com/cnap-tech/akua/tree/main';

/**
 * Rewrite source-tree links to URLs that resolve on the deployed
 * site. Source markdown lives under `docs/` / `examples/` / `skills/`
 * where relative paths resolve correctly inside the repo, but the
 * published site only mirrors a subset of those trees onto routes
 * like `/concepts/<slug>` and `/examples/<slug>`. Without rewriting
 * those links would 404.
 *
 * The renderer doesn't know the site layout itself; callers wire the
 * mappings via this options bag.
 */
export interface LinkResolverOpts {
	/** Repo-relative path of the markdown source being rendered (e.g.
	 *  `docs/cli-contract.md`, `examples/01-hello-webapp/README.md`).
	 *  Used to resolve `../foo` style relative links to a repo path. */
	sourceMd?: string;
	/** Map a `<name>.md` (without the `.md`) to a site URL, or return
	 *  null to fall back to the GitHub blob URL. */
	siteResolve?: (mdName: string) => string | null;
	/** Map a repo-relative non-md path (e.g. `examples/01-hello-webapp/`,
	 *  `skills/new-package/`) to a site URL. Return null to fall back
	 *  to the GitHub tree/blob URL. */
	repoResolve?: (repoPath: string) => string | null;
	/** Rewrite a bare in-page anchor (e.g. `#akua-render`) to a
	 *  full URL — used when the source markdown is one big doc that
	 *  the renderer split into per-page files. Return null to leave
	 *  the anchor untouched. */
	anchorResolve?: (anchor: string) => string | null;
}

/** Normalize a repo-relative path: collapse `./` and `..` segments. */
function normalizeRepoPath(p: string): string | null {
	const parts: string[] = [];
	for (const seg of p.split('/')) {
		if (seg === '' || seg === '.') continue;
		if (seg === '..') {
			if (parts.length === 0) return null;
			parts.pop();
		} else {
			parts.push(seg);
		}
	}
	return parts.join('/') + (p.endsWith('/') && parts.length > 0 ? '/' : '');
}

/** Resolve a relative href against the source markdown's directory,
 *  yielding a repo-rooted path. Returns null if the link escapes the
 *  repo root (which shouldn't happen for well-formed docs). */
function resolveAgainstSource(href: string, sourceMd: string | undefined): string | null {
	if (!sourceMd) return null;
	const sourceDir = sourceMd.includes('/') ? sourceMd.replace(/\/[^/]+$/, '') : '';
	const joined = sourceDir ? `${sourceDir}/${href}` : href;
	return normalizeRepoPath(joined);
}

export function rewriteUrl(url: string, opts: LinkResolverOpts = {}): string {
	if (url.startsWith('http') || url.startsWith('mailto:') || url.startsWith('tel:')) return url;

	// Bare in-page anchor.
	if (url.startsWith('#')) {
		if (opts.anchorResolve) {
			const resolved = opts.anchorResolve(url.slice(1));
			if (resolved) return resolved;
		}
		return url;
	}

	// Split off any fragment / query so path matching works on the
	// bare path.
	const hashIdx = url.indexOf('#');
	const queryIdx = url.indexOf('?');
	const cutAt = [hashIdx, queryIdx].filter((i) => i >= 0).sort((a, b) => a - b)[0];
	const path = cutAt !== undefined ? url.slice(0, cutAt) : url;
	const suffix = cutAt !== undefined ? url.slice(cutAt) : '';

	// Resolve to a repo-rooted path. Absolute `/x` is treated as
	// already repo-rooted; relative paths resolve against sourceMd's
	// directory.
	let repoPath: string | null;
	if (path.startsWith('/')) {
		repoPath = normalizeRepoPath(path.slice(1));
	} else {
		repoPath = resolveAgainstSource(path, opts.sourceMd);
	}

	// `.md` link — defer to siteResolve, fall back to GitHub blob.
	const mdMatch = (repoPath ?? path).match(/^(?:docs\/)?(.+)\.md$/);
	if (mdMatch) {
		const name = mdMatch[1].replace(/^errors\//, '');
		if (opts.siteResolve) {
			const resolved = opts.siteResolve(name);
			if (resolved) return `${resolved}${suffix}`;
		}
		const target = repoPath ?? `docs/${path}`;
		return `${GITHUB_BLOB}/${target}${suffix}`;
	}

	// Non-md link inside the repo — defer to repoResolve, fall back
	// to GitHub tree (for directories) or blob (for files).
	if (repoPath) {
		if (opts.repoResolve) {
			const resolved = opts.repoResolve(repoPath);
			if (resolved) return `${resolved}${suffix}`;
		}
		// Heuristic: trailing slash → directory → tree URL.
		const base = path.endsWith('/') ? GITHUB_TREE : GITHUB_BLOB;
		const cleaned = repoPath.replace(/\/$/, '');
		return `${base}/${cleaned}${suffix}`;
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
			// Slug rules match GitHub's: lowercase, strip
			// non-[a-z0-9 -], trim, then replace each space with a
			// single dash. Note `\s+` is wrong — it would collapse
			// consecutive spaces into one dash, but GitHub preserves
			// them (so `Foo — Bar` becomes `foo--bar`, not `foo-bar`).
			const slug = text
				.toLowerCase()
				.replace(/[^a-z0-9 -]/g, '')
				.trim()
				.replace(/ /g, '-');
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
