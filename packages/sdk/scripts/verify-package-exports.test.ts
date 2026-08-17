import { expect, test } from 'bun:test';

import { mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import {
	findMissingInTarball,
	findMissingOnDisk,
	parsePackResult,
	requiredExportPaths,
} from './verify-package-exports.ts';

// Regression fixture matching the actual shape of v0.9.3's broken
// package.json: `exports["./execute"]` promises `dist/execute.js`, which
// the bundler never produced because `src/execute.ts` was missing from its
// entrypoint list.
const SDK_LIKE_EXPORTS = {
	main: './dist/mod.js',
	types: './dist/mod.d.ts',
	exports: {
		'.': {
			types: './dist/mod.d.ts',
			default: './dist/mod.js',
		},
		'./execute': {
			types: './dist/execute.d.ts',
			default: './dist/execute.js',
		},
	},
};

test('requiredExportPaths collects every referenced file from main/types/exports, deduped and sorted', () => {
	const paths = requiredExportPaths(SDK_LIKE_EXPORTS);
	expect(paths).toEqual(['dist/execute.d.ts', 'dist/execute.js', 'dist/mod.d.ts', 'dist/mod.js']);
});

test('requiredExportPaths ignores non-relative and non-string values without throwing', () => {
	expect(requiredExportPaths(null)).toEqual([]);
	expect(requiredExportPaths('not-an-object')).toEqual([]);
	expect(requiredExportPaths({ exports: { '.': { node: 'some-package' } } })).toEqual([]);
});

test('findMissingOnDisk reproduces the v0.9.3 bug: execute.js absent while execute.d.ts exists', () => {
	const dir = mkdtempSync(join(tmpdir(), 'verify-package-exports-'));
	try {
		writeFileSync(join(dir, 'mod.js'), 'export {};');
		writeFileSync(join(dir, 'mod.d.ts'), 'export {};');
		// execute.d.ts exists (tsc's per-file declaration pass produced it)
		// but execute.js does not (the bundler never bundled src/execute.ts).
		writeFileSync(join(dir, 'execute.d.ts'), 'export {};');

		const required = requiredExportPaths(SDK_LIKE_EXPORTS).map((p) => p.replace(/^dist\//, ''));
		const missing = findMissingOnDisk(dir, required);

		expect(missing).toEqual(['execute.js']);
	} finally {
		rmSync(dir, { recursive: true, force: true });
	}
});

test('findMissingOnDisk reports nothing missing once every exports-mapped file exists', () => {
	const dir = mkdtempSync(join(tmpdir(), 'verify-package-exports-'));
	try {
		for (const file of ['mod.js', 'mod.d.ts', 'execute.js', 'execute.d.ts']) {
			writeFileSync(join(dir, file), 'export {};');
		}

		const required = requiredExportPaths(SDK_LIKE_EXPORTS).map((p) => p.replace(/^dist\//, ''));
		expect(findMissingOnDisk(dir, required)).toEqual([]);
	} finally {
		rmSync(dir, { recursive: true, force: true });
	}
});

test('parsePackResult reads a real `npm pack --dry-run --json` shape', () => {
	const npmPackOutput = [
		{
			files: [
				{ path: 'dist/mod.js' },
				{ path: 'dist/mod.d.ts' },
				{ path: 'dist/execute.d.ts' },
				{ path: 'README.md' },
			],
		},
	];
	const packed = parsePackResult(npmPackOutput[0]);
	expect(packed.files.map((f) => f.path)).toEqual([
		'dist/mod.js',
		'dist/mod.d.ts',
		'dist/execute.d.ts',
		'README.md',
	]);
});

test('parsePackResult tolerates malformed input instead of throwing', () => {
	expect(parsePackResult(undefined)).toEqual({ files: [] });
	expect(parsePackResult({ files: 'not-an-array' })).toEqual({ files: [] });
	expect(parsePackResult({ files: [{ notPath: 'x' }] })).toEqual({ files: [] });
});

test('findMissingInTarball flags an exports-mapped file the packed tarball never included', () => {
	const packed = parsePackResult({
		files: [{ path: 'dist/mod.js' }, { path: 'dist/mod.d.ts' }, { path: 'dist/execute.d.ts' }],
	});
	const required = requiredExportPaths(SDK_LIKE_EXPORTS);

	expect(findMissingInTarball(required, packed)).toEqual(['dist/execute.js']);
});

test('findMissingInTarball reports nothing missing once the tarball has every exports-mapped file', () => {
	const packed = parsePackResult({
		files: [
			{ path: 'dist/mod.js' },
			{ path: 'dist/mod.d.ts' },
			{ path: 'dist/execute.js' },
			{ path: 'dist/execute.d.ts' },
		],
	});
	const required = requiredExportPaths(SDK_LIKE_EXPORTS);

	expect(findMissingInTarball(required, packed)).toEqual([]);
});
