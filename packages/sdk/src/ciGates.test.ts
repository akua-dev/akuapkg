// Tests for the CI-gate trio: `check`, `lint`, `fmt`. WASM-backed —
// run under plain `task sdk:test`, no binary required.

import { describe, expect, test } from 'bun:test';
import { readFileSync, writeFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import ts from 'typescript';

import { Akua } from './mod.ts';
import {
	largeCrdPackageK,
	MINIMAL_AKUA_TOML,
	MINIMAL_PACKAGE_K,
	scratchPackageWith,
} from './test-utils.ts';

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), '../../..');
const loaderOnlyNapiExports = new Set(['setEnginesDir']);

function readRepo(path: string): string {
	return readFileSync(resolve(repoRoot, path), 'utf8');
}

function rustExportName(name: string): string {
	return name.replace(/_([a-z])/g, (_match, char: string) => char.toUpperCase());
}

function rustNapiFunctionNames(): string[] {
	const source = readRepo('crates/akua-napi/src/lib.rs');
	const names: string[] = [];
	for (const match of source.matchAll(
		/#\[napi(?:\([^\]]*\))?\](?:\s*#\[[^\]]+\])*\s*pub\s+(?:async\s+)?fn\s+([a-zA-Z0-9_]+)\(/g,
	)) {
		const name = match[1];
		if (name !== undefined) {
			names.push(rustExportName(name));
		}
	}
	return names.sort();
}

function dtsFunctionNames(): string[] {
	const source = readRepo('crates/akua-napi/index.d.ts');
	const names: string[] = [];
	for (const match of source.matchAll(/export declare function ([a-zA-Z0-9_]+)\(/g)) {
		const name = match[1];
		if (name !== undefined) {
			names.push(name);
		}
	}
	return names.sort();
}

function napiInterfaceFunctionNames(): string[] {
	const source = readRepo('packages/sdk/src/napi.ts');
	const sourceFile = ts.createSourceFile('napi.ts', source, ts.ScriptTarget.Latest, true);
	const names: string[] = [];

	function visit(node: ts.Node): void {
		if (ts.isInterfaceDeclaration(node) && node.name.text === 'NapiAddon') {
			for (const member of node.members) {
				if (ts.isMethodSignature(member) && ts.isIdentifier(member.name)) {
					names.push(member.name.text);
				}
			}
			return;
		}
		ts.forEachChild(node, visit);
	}

	visit(sourceFile);
	return names.sort();
}

function sdkNapiCallNames(): string[] {
	const mod = readRepo('packages/sdk/src/mod.ts');
	const names = new Set<string>();
	for (const match of mod.matchAll(/\bnapi\.([a-zA-Z0-9_]+)\(/g)) {
		const name = match[1];
		if (name !== undefined) {
			names.add(name);
		}
	}
	return [...names].sort();
}

describe('Akua CI-gate verbs', () => {
	test('check returns a CheckOutput with a "manifest" entry', async () => {
		using pkg = scratchPackageWith(MINIMAL_PACKAGE_K, 'akua-sdk-check-');
		writeFileSync(join(pkg.dir, 'akua.toml'), MINIMAL_AKUA_TOML);
		const akua = new Akua();
		const out = await akua.check({
			workspace: pkg.dir,
			package: join(pkg.dir, 'package.k'),
		});
		expect(['ok', 'fail']).toContain(out.status);
		expect(out.checks.some((c) => c.name === 'manifest')).toBe(true);
	});

	test('lint returns a LintOutput with an issues array', async () => {
		using pkg = scratchPackageWith(MINIMAL_PACKAGE_K, 'akua-sdk-lint-');
		const akua = new Akua();
		const out = await akua.lint({ package: join(pkg.dir, 'package.k') });
		expect(['ok', 'fail']).toContain(out.status);
		expect(Array.isArray(out.issues)).toBe(true);
	});

	test('fmt --check reports whether formatting would change the file', async () => {
		using pkg = scratchPackageWith(MINIMAL_PACKAGE_K, 'akua-sdk-fmt-');
		const akua = new Akua();
		const out = await akua.fmt({
			package: join(pkg.dir, 'package.k'),
			check: true,
		});
		expect(out.files.length).toBe(1);
		expect(typeof out.files[0].changed).toBe('boolean');
	});

	test('export emits JSON Schema 2020-12 with @ui decorators projected to x-ui', async () => {
		const PKG_WITH_UI = `
schema Input:
    """Public inputs."""

    @ui(order=10, group="Identity")
    name: str = "hello"

    replicas: int = 2

resources = []
`;
		using pkg = scratchPackageWith(PKG_WITH_UI, 'akua-sdk-export-');
		const akua = new Akua();
		const schema = await akua.export({ package: join(pkg.dir, 'package.k') });
		expect(schema.$schema).toBe('https://json-schema.org/draft/2020-12/schema');
		const props = schema.properties as Record<string, Record<string, unknown>>;
		expect(props.name.type).toBe('string');
		const xUi = props.name['x-ui'] as Record<string, unknown>;
		expect(xUi.order).toBe(10);
		expect(xUi.group).toBe('Identity');
	});

	test('export with format=openapi wraps Input under components.schemas', async () => {
		using pkg = scratchPackageWith(MINIMAL_PACKAGE_K, 'akua-sdk-export-openapi-');
		const akua = new Akua();
		const doc = await akua.export({
			package: join(pkg.dir, 'package.k'),
			format: 'openapi',
		});
		expect(doc.openapi).toBe('3.1.0');
		const components = doc.components as Record<string, Record<string, unknown>>;
		expect(typeof components.schemas).toBe('object');
		expect(components.schemas.Input).toBeDefined();
	});

	test('export remains schema-only for large resource bodies', async () => {
		using pkg = scratchPackageWith(largeCrdPackageK(64, 4096), 'akua-sdk-large-export-');
		const akua = new Akua();
		const doc = await akua.export({
			package: join(pkg.dir, 'package.k'),
			format: 'openapi',
		});

		expect(doc.openapi).toBe('3.1.0');
		const serialized = JSON.stringify(doc);
		expect(serialized.length).toBeLessThan(20_000);
		expect(serialized).not.toContain('widgets-63.example.com');
	});
});

describe('SDK/NAPI drift guards', () => {
	test('drift: Rust napi exports match committed index.d.ts and SDK-callable NapiAddon', () => {
		const rust = rustNapiFunctionNames();
		const dts = dtsFunctionNames();
		const addon = napiInterfaceFunctionNames();

		expect(dts).toEqual(rust);
		for (const name of loaderOnlyNapiExports) {
			expect(rust).toContain(name);
		}
		// Loader-only exports are consumed by crates/akua-napi/loader.js
		// before the SDK receives the addon. Public SDK call sites are
		// guarded separately below.
		const sdkCallableRust = rust.filter((name) => !loaderOnlyNapiExports.has(name));
		expect(addon).toEqual(sdkCallableRust);
	});

	test('drift: public SDK methods that call napi have a matching addon binding', () => {
		expect(sdkNapiCallNames()).toEqual(napiInterfaceFunctionNames());
	});
});
