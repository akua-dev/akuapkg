import { expect, test } from 'bun:test';

import { resolve } from 'node:path';

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
	expect(stdout.trim()).toBe('embedded\n0');
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
	expect(stdout.trimEnd()).toEndWith('0');
});
