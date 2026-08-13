//! Shared HTML shell + sidebar primitives for every site generator.
//!
//! Page composition is the same across `/errors/<CODE>`, `/cli/<verb>`,
//! `/concepts/<topic>`, and `/examples/<name>`:
//!
//!   ┌─────────────────────────────────────────────────────────┐
//!   │ logo + wordmark      ·  Start · CLI · Concepts · …      │  topnav
//!   ├──────────────┬──────────────────────────────────────────┤
//!   │              │                                          │
//!   │  per-section │  page body (markdown-rendered)           │
//!   │  sidebar     │                                          │
//!   │              │                                          │
//!   └──────────────┴──────────────────────────────────────────┘
//!
//! The sidebar collapses below 880px. Top nav stays visible on mobile.

// ---------------------------------------------------------------------------
// Sidebar spec — generators build one of these and hand it to pageShell.
// ---------------------------------------------------------------------------

export interface SidebarItem {
	/** Absolute path the link points at. */
	href: string;
	/** What the user reads. */
	label: string;
	/** Highlight as the current page. */
	active?: boolean;
}

export interface SidebarSection {
	/** Section title above the items. Omit for an unlabeled flat list. */
	title?: string;
	items: SidebarItem[];
}

export interface SidebarSpec {
	/** Top-level link rendered above the section list ("All error codes"). */
	rootHref: string;
	rootLabel: string;
	sections: SidebarSection[];
}

// ---------------------------------------------------------------------------
// Top-nav spec — same on every page.
// ---------------------------------------------------------------------------

export const TOPNAV: { href: string; label: string }[] = [
	{ href: '/start/', label: 'Start' },
	{ href: '/cli/', label: 'CLI' },
	{ href: '/concepts/', label: 'Concepts' },
	{ href: '/examples/', label: 'Examples' },
	{ href: '/errors/', label: 'Errors' },
];

// ---------------------------------------------------------------------------
// Escaping
// ---------------------------------------------------------------------------

export function escape(s: string): string {
	return s
		.replace(/&/g, '&amp;')
		.replace(/</g, '&lt;')
		.replace(/>/g, '&gt;')
		.replace(/"/g, '&quot;');
}

// ---------------------------------------------------------------------------
// Style — the single source of truth for the site's visual identity.
//
// Palette: dark charcoal background (matches the logo's dark gear),
// muted greys for non-essential text, the existing red accent for
// active states / CTAs. Logo introduces blue but it's contained to
// the mark and doesn't propagate.
// ---------------------------------------------------------------------------

const STYLE = `
<style>
:root {
  color-scheme: dark;
  --bg: #0a0a0a;
  --fg: #e8e8ea;
  --muted: #8a8a8d;
  --line: #1b1b1d;
  --line-strong: #2a2a2d;
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
}

/* ----- Top nav -----
 * Fixed-position bar that fades in as the user scrolls past the top.
 * Driven by an inline script that toggles a "scrolled" class on body. */
.topnav {
  position: fixed;
  top: 0; left: 0; right: 0;
  z-index: 10;
  background: rgba(10, 10, 10, 0.85);
  backdrop-filter: blur(12px);
  -webkit-backdrop-filter: blur(12px);
  border-bottom: 1px solid var(--line);
  transform: translateY(-100%);
  opacity: 0;
  transition: transform 220ms ease-out, opacity 220ms ease-out;
  pointer-events: none;
}
body.scrolled .topnav {
  transform: translateY(0);
  opacity: 1;
  pointer-events: auto;
}
.topnav-inner {
  display: flex;
  align-items: center;
  gap: 32px;
  max-width: 1080px;
  margin: 0 auto;
  padding: 12px 20px;
  min-height: 52px;
}
.brand {
  display: flex;
  align-items: center;
  gap: 10px;
  text-decoration: none;
  color: var(--fg);
  font-weight: 600;
  letter-spacing: -0.01em;
  line-height: 1;
}
.brand img { width: 26px; height: 26px; display: block; }
.brand span { font-size: 16px; line-height: 1; }
.brand:hover { text-decoration: none; }
.topnav-links {
  display: flex;
  align-items: center;
  gap: 22px;
  margin: 0 0 0 auto;
  padding: 0;
  list-style: none;
  line-height: 1;
}
.topnav-links li { line-height: 1; }
.topnav-links a {
  display: inline-flex;
  align-items: center;
  height: 28px;
  color: var(--muted);
  text-decoration: none;
  font-size: 14px;
  line-height: 1;
  letter-spacing: 0.01em;
  transition: color 120ms ease;
}
.topnav-links a:hover, .topnav-links a.current { color: var(--fg); }

/* ----- Page layout ----- */
.layout {
  display: grid;
  grid-template-columns: 240px minmax(0, 720px);
  gap: 56px;
  max-width: 1080px;
  margin: 0 auto;
  padding: 48px 20px;
  align-items: start;
}
.layout.no-sidebar { grid-template-columns: minmax(0, 720px); margin: 0 auto; }

/* Landing layout: single centered column, no padding inflation from
 * a missing sidebar slot. The vertical-centered feel comes from the
 * grid centering on body, not from a hero block above the content. */
body.landing {
  display: grid;
  grid-template-rows: 1fr;
  align-items: center;
  min-height: 100vh;
}
body.landing .layout {
  grid-template-columns: minmax(0, 560px);
  padding: 60px 20px;
}

/* ----- Sidebar -----
 * Sticky at top: 24px so it floats just under the (fade-in) nav when
 * one is showing, and reads cleanly at the very top when it isn't. */
.sidebar {
  position: sticky;
  top: 24px;
  max-height: calc(100vh - 48px);
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
}
.sidebar-title a {
  color: var(--muted);
  text-decoration: none;
}
.sidebar-title a:hover { color: var(--fg); }
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

/* ----- Content ----- */
main { width: 100%; max-width: 720px; min-width: 0; }
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
header .tagline { margin: 8px 0 0; color: var(--muted); font-size: 16px; }
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
table {
  width: 100%;
  border-collapse: collapse;
  margin: 0 0 16px;
  font-size: 14px;
}
th, td {
  text-align: left;
  padding: 8px 12px;
  border-bottom: 1px solid var(--line);
}
th { color: var(--muted); font-weight: 600; font-size: 13px; letter-spacing: 0.02em; }
hr { border: 0; border-top: 1px solid var(--line); margin: 32px 0; }
footer { margin-top: 64px; padding-top: 16px; border-top: 1px solid var(--line); color: var(--muted); font-size: 13px; }

/* ----- Generic page-element classes used by gen-* generators ----- */
.code-list { list-style: none; padding: 0; margin: 0; }
.code-list li { margin: 0 0 14px; padding: 0; }
.code-list .name { font: 14px/1 ui-monospace, SFMono-Regular, Menlo, Consolas, monospace; }
.code-list .summary { color: var(--muted); font-size: 14px; margin-top: 4px; }
.section-group { margin-bottom: 32px; }
.section-group h2 { margin-bottom: 14px; }

/* ----- Mobile ----- */
@media (max-width: 880px) {
  .layout { grid-template-columns: 1fr; gap: 0; }
  .sidebar { display: none; }
  .topnav-inner { gap: 16px; padding: 12px 16px; }
  .topnav-links { gap: 14px; }
  .topnav-links a { font-size: 13px; }
}
@media (max-width: 520px) {
  .topnav-links { display: none; }
}
</style>
`.trim();

// ---------------------------------------------------------------------------
// HTML rendering
// ---------------------------------------------------------------------------

function renderTopnav(currentSection: string | null): string {
	const links = TOPNAV.map((nav) => {
		const cls = nav.href === currentSection ? ' class="current"' : '';
		return `<li><a href="${nav.href}"${cls}>${escape(nav.label)}</a></li>`;
	}).join('');
	return `<nav class="topnav">
<div class="topnav-inner">
  <a class="brand" href="/">
    <img src="/assets/logo.png" alt="" />
    <span>akua</span>
  </a>
  <ul class="topnav-links">${links}</ul>
</div>
</nav>`;
}

function renderSidebar(spec: SidebarSpec): string {
	const sections = spec.sections
		.map((section) => {
			const li = section.items
				.map((item) => {
					const cls = item.active ? ' class="active"' : '';
					return `<li><a href="${item.href}"${cls}>${escape(item.label)}</a></li>`;
				})
				.join('');
			const title = section.title ? `<h3>${escape(section.title)}</h3>` : '';
			return `${title}<ul>${li}</ul>`;
		})
		.join('\n');
	return `<aside class="sidebar">
<p class="sidebar-title"><a href="${spec.rootHref}">${escape(spec.rootLabel)}</a></p>
${sections}
</aside>`;
}

export interface PageOpts {
	title: string;
	description: string;
	body: string;
	/** Which top-nav entry to highlight, or null for the landing. */
	currentSection: string | null;
	/** Sidebar spec, or null for full-width layouts (landing, /start). */
	sidebar: SidebarSpec | null;
	/** Canonical URL used by OG metadata and, when enabled, a canonical link. */
	canonicalUrl?: string;
	/** Emit a canonical link tag in addition to the OG URL. */
	emitCanonicalLink?: boolean;
	/** Extra body class (e.g. `landing` for vertical-centered hero). */
	bodyClass?: string;
}

/** Inline script that toggles `body.scrolled` past 40px so the topnav
 * fades in. Tiny — keeps the site dependency-free. */
const SCROLL_SCRIPT = `<script>
(function () {
  var threshold = 40;
  function update() {
    document.body.classList.toggle('scrolled', window.scrollY > threshold);
  }
  update();
  addEventListener('scroll', update, { passive: true });
})();
</script>`;

export function pageShell(opts: PageOpts): string {
	const ogUrl = opts.canonicalUrl ?? 'https://akua.dev/';
	const canonicalLink = opts.emitCanonicalLink
		? `<link rel="canonical" href="${escape(ogUrl)}" />\n`
		: '';
	const sidebarHtml = opts.sidebar ? renderSidebar(opts.sidebar) : '';
	const layoutClass = opts.sidebar ? 'layout' : 'layout no-sidebar';
	const bodyClass = opts.bodyClass ?? '';
	return `<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8" />
<meta name="viewport" content="width=device-width, initial-scale=1" />
<title>${escape(opts.title)} — akua</title>
<meta name="description" content="${escape(opts.description)}" />
<meta name="color-scheme" content="dark" />
<link rel="icon" type="image/png" href="/assets/logo.png" />
<link rel="apple-touch-icon" href="/assets/logo.png" />
${canonicalLink}<meta property="og:title" content="${escape(opts.title)} — akua" />
<meta property="og:description" content="${escape(opts.description)}" />
<meta property="og:url" content="${escape(ogUrl)}" />
<meta property="og:image" content="https://akua.dev/assets/logo.png" />
<meta name="twitter:card" content="summary" />
${STYLE}
</head>
<body${bodyClass ? ` class="${bodyClass}"` : ''}>
${renderTopnav(opts.currentSection)}
<div class="${layoutClass}">
${sidebarHtml}
<main>
${opts.body}
<footer>
  <a href="/">akua.dev</a> ·
  <a href="https://github.com/akua-dev/akuapkg">github.com/akua-dev/akuapkg</a> ·
  Apache-2.0
</footer>
</main>
</div>
${SCROLL_SCRIPT}
</body>
</html>
`;
}

// ---------------------------------------------------------------------------
// stripTags — used by every generator to extract plain-text descriptions
// from rendered HTML. Unescapes entities so downstream `escape()` passes
// don't double-encode (`&lt;` → `&amp;lt;`).
// ---------------------------------------------------------------------------

export function stripTags(html: string): string {
	return html
		.replace(/<[^>]+>/g, '')
		.replace(/&lt;/g, '<')
		.replace(/&gt;/g, '>')
		.replace(/&quot;/g, '"')
		.replace(/&#39;/g, "'")
		.replace(/&amp;/g, '&')
		.replace(/\s+/g, ' ')
		.trim();
}
