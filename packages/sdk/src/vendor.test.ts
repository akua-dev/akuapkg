import { existsSync, mkdirSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';

import { describe, expect, test } from 'bun:test';

import { Akua } from './mod.ts';
import { minimalAkuaToml, scratchPackage } from './test-utils.ts';

const akua = new Akua();

function writeVendorWorkspace(dir: string) {
	writeFileSync(
		join(dir, 'akua.toml'),
		`${minimalAkuaToml('vendor-sdk-test')}\n[dependencies]\nlocal = { path = "./charts/local" }\n`,
	);
	const chart = join(dir, 'charts/local');
	mkdirSync(join(chart, 'templates'), { recursive: true });
	writeFileSync(join(chart, 'Chart.yaml'), 'apiVersion: v2\nname: local\nversion: 0.1.0\n');
	writeFileSync(join(chart, 'templates/cm.yaml'), 'kind: ConfigMap\n');
	writeFileSync(
		join(dir, 'package.k'),
		'schema Input:\n    x: int = 1\n\ninput: Input = option("input") or Input {}\n\nresources = []\n',
	);
}

describe('Akua.vendorAdd', () => {
	test('writes the vendor tree for a declared path dependency', async () => {
		using ws = scratchPackage('akua-sdk-vendor-add-');
		writeVendorWorkspace(ws.dir);

		const out = await akua.vendorAdd('local', { workspace: ws.dir });
		expect(out.name).toBe('local');
		expect(out.wrote).toBe(true);
		expect(out.path).toContain('.akua/vendor/local');
		expect(existsSync(join(ws.dir, '.akua/vendor/local'))).toBe(true);

		const listed = await akua.vendorList({ workspace: ws.dir });
		expect(listed.entries).toHaveLength(1);
		expect(listed.entries[0].orphan).toBe(false);

		const checked = await akua.vendorCheck({ workspace: ws.dir });
		expect(checked.drift).toBe(false);
		expect(checked.entries.some((entry) => entry.name === 'local')).toBe(true);
	});

	test('plan mode does not mutate the workspace', async () => {
		using ws = scratchPackage('akua-sdk-vendor-plan-');
		writeVendorWorkspace(ws.dir);

		const out = await akua.vendorAdd('local', { workspace: ws.dir, plan: true });
		expect(out.name).toBe('local');
		expect(existsSync(join(ws.dir, '.akua/vendor/local'))).toBe(false);
	});
});

describe('Akua.vendorList / Akua.vendorCheck', () => {
	test('orphaned vendor trees are surfaced and drift stays typed', async () => {
		using ws = scratchPackage('akua-sdk-vendor-list-');
		writeVendorWorkspace(ws.dir);

		await akua.vendorAdd('local', { workspace: ws.dir });
		const orphan = join(ws.dir, '.akua/vendor/orphan');
		mkdirSync(join(orphan, 'templates'), { recursive: true });
		writeFileSync(join(orphan, 'Chart.yaml'), 'apiVersion: v2\nname: orphan\nversion: 0.1.0\n');
		writeFileSync(join(orphan, 'templates/cm.yaml'), 'kind: ConfigMap\n');

		const listed = await akua.vendorList({ workspace: ws.dir });
		const orphanEntry = listed.entries.find((entry) => entry.name === 'orphan');
		expect(orphanEntry?.orphan).toBe(true);

		writeFileSync(join(ws.dir, '.akua/vendor/local/templates/cm.yaml'), 'kind: Secret\n');
		const checked = await akua.vendorCheck({ workspace: ws.dir });
		expect(checked.drift).toBe(true);
		expect(checked.orphaned).toContain('orphan');
		expect(checked.entries.some((entry) => entry.name === 'local')).toBe(true);
	});

	test('missing dep surfaces the structured vendor error', async () => {
		using ws = scratchPackage('akua-sdk-vendor-missing-');
		writeFileSync(
			join(ws.dir, 'akua.toml'),
			`${minimalAkuaToml('vendor-sdk-test')}\n[dependencies]\n`,
		);
		writeFileSync(join(ws.dir, 'package.k'), 'resources = []\n');

		await expect(akua.vendorAdd('ghost', { workspace: ws.dir })).rejects.toMatchObject({
			structured: { code: 'E_VENDOR_DEP_MISSING' },
		});
	});
});
