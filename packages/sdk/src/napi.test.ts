import { expect, test } from 'bun:test';

import { resolve } from 'node:path';

test('exposes package execution without loading the schema-validation client', async () => {
	const script = resolve(import.meta.dir, 'napi-execute-subpath-child.ts');
	const proc = Bun.spawn([process.execPath, script], {
		stdout: 'pipe',
		stderr: 'pipe',
	});
	const [exitCode, stdout, stderr] = await Promise.all([
		proc.exited,
		new Response(proc.stdout).text(),
		new Response(proc.stderr).text(),
	]);

	expect(exitCode).toBe(0);
	expect(stderr).toBe('');
	expect(stdout).toContain('"version"');
	expect(stdout.trimEnd()).toEndWith('0');

	const packageJson = await Bun.file(resolve(import.meta.dir, '../package.json')).json();
	expect(packageJson.exports['./execute']).toEqual({
		types: './dist/execute.d.ts',
		default: './dist/execute.js',
	});
	const moduleSource = await Bun.file(resolve(import.meta.dir, 'execute.ts')).text();
	expect(moduleSource).not.toContain('validate');
	expect(moduleSource).not.toContain('ajv');
});

test('uses an explicitly configured native addon before resolving packages', async () => {
	const script = resolve(import.meta.dir, 'napi-configure-child.ts');
	const proc = Bun.spawn([process.execPath, script], {
		stdout: 'pipe',
		stderr: 'pipe',
	});
	const [exitCode, stdout, stderr] = await Promise.all([
		proc.exited,
		new Response(proc.stdout).text(),
		new Response(proc.stderr).text(),
	]);

	expect(exitCode).toBe(0);
	expect(stderr).toBe('');
	expect(stdout.trim()).toBe(
		'embedded\n0\n[{"args":["version"],"options":{"binName":"akua pkg"}}]',
	);
});

test('runs the complete package command dispatcher through the built native addon', async () => {
	const script = resolve(import.meta.dir, 'napi-execute-child.ts');
	const proc = Bun.spawn([process.execPath, script], {
		stdout: 'pipe',
		stderr: 'pipe',
	});
	const [exitCode, stdout, stderr] = await Promise.all([
		proc.exited,
		new Response(proc.stdout).text(),
		new Response(proc.stderr).text(),
	]);

	expect(exitCode).toBe(0);
	expect(stderr).toBe('');
	expect(stdout).toContain('"version"');
	expect(stdout).toContain('Usage: akua pkg render [OPTIONS]');
	expect(stdout).not.toContain('Usage: akuapkg render');
	expect(stdout.trimEnd()).toEndWith('0');
});
