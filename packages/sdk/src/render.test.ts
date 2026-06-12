// `Akua.render(opts)` — exercises the napi addon in-process.
// No binary, no shell-out, no E2E env-var gate.

import { describe, expect, test } from 'bun:test';
import { join } from 'node:path';
import { writeFileSync } from 'node:fs';

import { Akua } from './mod.ts';
import {
	largeCrdPackageK,
	MINIMAL_AKUA_TOML,
	MINIMAL_PACKAGE_K,
	scratchPackage,
	scratchPackageWith,
} from './test-utils.ts';

describe('Akua.render', () => {
	test('renders a minimal package and returns a typed summary', async () => {
		using pkg = scratchPackage('akua-sdk-render-');
		const pkgPath = join(pkg.dir, 'package.k');
		writeFileSync(pkgPath, MINIMAL_PACKAGE_K);
		writeFileSync(join(pkg.dir, 'akua.toml'), MINIMAL_AKUA_TOML);
		const akua = new Akua();
		const summary = await akua.render({
			package: pkgPath,
			out: join(pkg.dir, 'deploy'),
		});
		expect(summary.format).toBe('raw-manifests');
		expect(summary.manifests).toBe(1);
		expect(summary.hash).toMatch(/^sha256:/);
		expect(summary.target).toContain('deploy');
	});

	test('--dry-run skips writing but still returns a summary', async () => {
		using pkg = scratchPackage('akua-sdk-render-');
		const pkgPath = join(pkg.dir, 'package.k');
		writeFileSync(pkgPath, MINIMAL_PACKAGE_K);
		writeFileSync(join(pkg.dir, 'akua.toml'), MINIMAL_AKUA_TOML);
		const akua = new Akua();
		const summary = await akua.render({
			package: pkgPath,
			out: join(pkg.dir, 'deploy'),
			dryRun: true,
		});
		expect(summary.manifests).toBe(1);
	});

	test('large packages return a compact RenderSummary', async () => {
		using pkg = scratchPackageWith(largeCrdPackageK(256), 'akua-sdk-large-render-');
		const pkgPath = join(pkg.dir, 'package.k');
		writeFileSync(join(pkg.dir, 'akua.toml'), MINIMAL_AKUA_TOML);

		const akua = new Akua();
		const dry = await akua.render({
			package: pkgPath,
			out: join(pkg.dir, 'deploy'),
			dryRun: true,
		});

		expect(dry.manifests).toBe(256);
		expect(JSON.stringify(dry).length).toBeLessThan(40_000);
		expect('resources' in dry).toBe(false);
	});
});
