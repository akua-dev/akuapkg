#!/usr/bin/env bun
// Fails the build/pack step if package.json's `main`, `types`, or `exports`
// map reference a file the build didn't actually produce.
//
// v0.9.3 shipped with `exports["./execute"].default` pointing at
// `dist/execute.js`, which never existed: the bundler's entrypoint list
// only included `src/mod.ts`, but `bunx tsc --emitDeclarationOnly` emits a
// per-source `.d.ts` for every file under `src/` regardless of what the
// bundler was told to build, so `dist/execute.d.ts` existed and masked the
// gap. `npm pack --dry-run` didn't catch it either — it only reports what
// IS in the tarball, not what the manifest promises should be there.
//
// Run with no arguments to check a built `dist/` on disk (used right after
// `bun build`, see package.json's `build` script and `task sdk:build`).
// Run with `--tarball <path>` against the JSON `npm pack --dry-run --json`
// writes, to additionally verify the packed tarball — not just the local
// `dist/` — actually contains every exports-mapped file (used by
// `task sdk:publish:check` and the release workflow's pack step).

import { existsSync } from 'node:fs';
import { resolve } from 'node:path';

interface PackedFile {
	path: string;
}

interface PackResult {
	files: PackedFile[];
}

function isRecord(value: unknown): value is Record<string, unknown> {
	return typeof value === 'object' && value !== null && !Array.isArray(value);
}

/** Recursively collects every `./`-relative file path referenced anywhere
 * in an exports-map-shaped value: plain strings, nested condition objects
 * (`{ types, default, import, require, ... }`), or arrays of either. */
function collectRelativePaths(node: unknown, out: Set<string>): void {
	if (typeof node === 'string') {
		if (node.startsWith('./')) out.add(node.slice(2));
		return;
	}
	if (Array.isArray(node)) {
		for (const item of node) collectRelativePaths(item, out);
		return;
	}
	if (isRecord(node)) {
		for (const value of Object.values(node)) collectRelativePaths(value, out);
	}
}

/** Every file package.json's `main`, `types`, `module`, and `exports`
 * fields claim exist, as paths relative to the package root. */
export function requiredExportPaths(pkg: unknown): string[] {
	if (!isRecord(pkg)) return [];
	const paths = new Set<string>();
	for (const field of ['main', 'types', 'module']) {
		const value = pkg[field];
		if (typeof value === 'string') collectRelativePaths(value, paths);
	}
	if ('exports' in pkg) collectRelativePaths(pkg.exports, paths);
	return [...paths].sort();
}

/** Required paths that don't exist on disk under `pkgDir`. */
export function findMissingOnDisk(pkgDir: string, required: string[]): string[] {
	return required.filter((relPath) => !existsSync(resolve(pkgDir, relPath)));
}

function isPackedFile(value: unknown): value is PackedFile {
	return isRecord(value) && typeof value.path === 'string';
}

/** Parses one entry of `npm pack --dry-run --json` output into the file
 * list it packed. Malformed/unexpected shapes parse to an empty file list
 * so callers see every required path reported missing, rather than the
 * check silently passing on a shape it doesn't understand. */
export function parsePackResult(value: unknown): PackResult {
	if (!isRecord(value)) return { files: [] };
	const filesRaw = value.files;
	if (!Array.isArray(filesRaw)) return { files: [] };
	return { files: filesRaw.filter(isPackedFile) };
}

/** Required paths that aren't present in a packed tarball's file list. */
export function findMissingInTarball(required: string[], packed: PackResult): string[] {
	const present = new Set(packed.files.map((f) => f.path));
	return required.filter((relPath) => !present.has(relPath));
}

async function main(): Promise<void> {
	const pkgDir = resolve(import.meta.dir, '..');
	const pkg: unknown = await Bun.file(resolve(pkgDir, 'package.json')).json();
	const required = requiredExportPaths(pkg);
	if (required.length === 0) {
		console.error(
			'verify-package-exports: found no main/types/exports paths in package.json — refusing to pass trivially',
		);
		process.exit(1);
	}

	const tarballFlagIndex = process.argv.indexOf('--tarball');
	if (tarballFlagIndex !== -1) {
		const jsonPath = process.argv[tarballFlagIndex + 1];
		if (!jsonPath) {
			console.error('verify-package-exports: --tarball requires a path to `npm pack --dry-run --json` output');
			process.exit(1);
		}
		const packedRaw: unknown = await Bun.file(jsonPath).json();
		const first = Array.isArray(packedRaw) ? packedRaw[0] : undefined;
		const missing = findMissingInTarball(required, parsePackResult(first));
		if (missing.length > 0) {
			console.error(
				`verify-package-exports: packed tarball is missing files required by package.json's exports map:\n${missing.map((p) => `  - ${p}`).join('\n')}`,
			);
			process.exit(1);
		}
		console.log(`verify-package-exports: tarball contains all ${required.length} exports-mapped file(s).`);
		return;
	}

	const missing = findMissingOnDisk(pkgDir, required);
	if (missing.length > 0) {
		console.error(
			`verify-package-exports: build is missing files required by package.json's exports map:\n${missing.map((p) => `  - ${p}`).join('\n')}`,
		);
		process.exit(1);
	}
	console.log(`verify-package-exports: dist/ contains all ${required.length} exports-mapped file(s).`);
}

if (import.meta.main) {
	await main();
}
