import { describe, expect, test } from 'bun:test';
import { createHash } from 'node:crypto';
import { mkdtemp, readFile, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { create as createTar } from 'tar';

import { Akua } from './mod.ts';

const AKUA_PACKAGE_LAYER_MEDIA_TYPE = 'application/vnd.akua.package.content.v1.tar+gzip';
const OCI_MANIFEST_MEDIA_TYPE = 'application/vnd.oci.image.manifest.v1+json';

function sha256(bytes: Uint8Array | string): string {
	return `sha256:${createHash('sha256').update(bytes).digest('hex')}`;
}

async function packageTarball(): Promise<Buffer> {
	const dir = await mkdtemp(join(tmpdir(), 'akua-sdk-oci-'));
	await writeFile(
		join(dir, 'akua.toml'),
		'[package]\nname = "sdk-demo"\nversion = "1.2.3"\nedition = "akua.dev/v1alpha1"\n',
	);
	await writeFile(
		join(dir, 'package.k'),
		'schema Input:\n    license_key: str\n\ninput: Input = option("input") or Input {}\nresources = []\n',
	);
	const tarballPath = join(dir, 'package.tar.gz');
	await createTar(
		{
			cwd: dir,
			file: tarballPath,
			gzip: true,
		},
		['akua.toml', 'package.k'],
	);
	return readFile(tarballPath);
}

async function readFirstLine(stream: ReadableStream<Uint8Array>): Promise<string> {
	const reader = stream.getReader();
	const decoder = new TextDecoder();
	let buffered = '';
	while (true) {
		const { done, value } = await reader.read();
		if (done) {
			return buffered.trim();
		}
		buffered += decoder.decode(value, { stream: true });
		const newline = buffered.indexOf('\n');
		if (newline !== -1) {
			return buffered.slice(0, newline).trim();
		}
	}
}

async function startRegistryServer(args: {
	manifest: string;
	tarball: Buffer;
	layerDigest: string;
}): Promise<{ registry: string; stop: () => void }> {
	const dir = await mkdtemp(join(tmpdir(), 'akua-sdk-oci-server-'));
	const script = join(dir, 'server.js');
	await writeFile(
		script,
		`
const tarball = Buffer.from(process.env.TARBALL_B64, 'base64');
const manifest = process.env.MANIFEST;
const layerDigest = process.env.LAYER_DIGEST;
const server = Bun.serve({
  hostname: '127.0.0.1',
  port: 0,
  fetch(req) {
    const url = new URL(req.url);
    if (req.headers.get('authorization') !== 'Bearer package-token') {
      return new Response('missing auth', { status: 404 });
    }
    if (url.pathname === '/v2/team/sdk-demo/manifests/1.2.3') {
      return new Response(manifest, {
        headers: { 'content-type': '${OCI_MANIFEST_MEDIA_TYPE}' },
      });
    }
    if (url.pathname === '/v2/team/sdk-demo/blobs/' + layerDigest) {
      return new Response(tarball, {
        headers: { 'content-type': '${AKUA_PACKAGE_LAYER_MEDIA_TYPE}' },
      });
    }
    return new Response('not found', { status: 404 });
  },
});
console.log(server.port);
process.on('SIGTERM', () => {
  server.stop(true);
  process.exit(0);
});
await new Promise(() => {});
`,
	);
	const proc = Bun.spawn([process.execPath, script], {
		stdout: 'pipe',
		stderr: 'pipe',
		env: {
			...process.env,
			MANIFEST: args.manifest,
			TARBALL_B64: args.tarball.toString('base64'),
			LAYER_DIGEST: args.layerDigest,
		},
	});
	const port = await readFirstLine(proc.stdout);
	return {
		registry: `127.0.0.1:${port}`,
		stop: () => {
			proc.kill();
		},
	};
}

describe('Akua.inspectOciPackage', () => {
	test('requires ociRef and tag', async () => {
		const akua = new Akua();

		await expect(
			akua.inspectOciPackage({ ociRef: '', tag: '1.2.3' }),
		).rejects.toThrow('inspectOciPackage: ociRef is required');
		await expect(
			akua.inspectOciPackage({
				ociRef: 'oci://example.invalid/team/pkg',
				tag: '',
			}),
		).rejects.toThrow('inspectOciPackage: tag is required');
	});

	test('validates explicit OCI auth entries before loading native code', async () => {
		const akua = new Akua();

		await expect(
			akua.inspectOciPackage({
				ociRef: 'oci://example.invalid/team/pkg',
				tag: '1.2.3',
				auth: {
					'  ': { username: 'user', password: 'pass' },
				},
			}),
		).rejects.toThrow('inspectOciPackage: auth registry key is required');
		await expect(
			akua.inspectOciPackage({
				ociRef: 'oci://example.invalid/team/pkg',
				tag: '1.2.3',
				auth: {
					'example.invalid': { token: '' },
				},
			}),
		).rejects.toThrow('inspectOciPackage: auth token is required for example.invalid');
		await expect(
			akua.inspectOciPackage({
				ociRef: 'oci://example.invalid/team/pkg',
				tag: '1.2.3',
				auth: {
					'example.invalid': { username: 'user', password: '' },
				},
			}),
		).rejects.toThrow(
			'inspectOciPackage: username and password are required for example.invalid',
		);
	});

	test('inspects a published OCI package through the native SDK boundary', async () => {
		const akua = new Akua();
		const tarball = await packageTarball();
		const layerDigest = sha256(tarball);
		const manifest = JSON.stringify({
			schemaVersion: 2,
			mediaType: OCI_MANIFEST_MEDIA_TYPE,
			config: {
				mediaType: 'application/vnd.akua.package.config.v1+json',
				digest: sha256('{}'),
				size: 2,
			},
			layers: [
				{
					mediaType: AKUA_PACKAGE_LAYER_MEDIA_TYPE,
					digest: layerDigest,
					size: tarball.byteLength,
				},
			],
		});
		const manifestDigest = sha256(manifest);
		const server = await startRegistryServer({ manifest, tarball, layerDigest });

		try {
			const inspected = await akua.inspectOciPackage({
				ociRef: `oci://${server.registry}/team/sdk-demo`,
				tag: '1.2.3',
				auth: {
					[server.registry]: { token: 'package-token' },
				},
			});

			expect(inspected.kind).toBe('oci_package');
			expect(inspected.package_name).toBe('sdk-demo');
			expect(inspected.package_version).toBe('1.2.3');
			expect(inspected.layer_digest).toBe(layerDigest);
			expect(inspected.manifest_digest).toBe(manifestDigest);
			expect(inspected.input_schema.type).toBe('object');
			expect(inspected.input_schema.properties).toMatchObject({
				license_key: { type: 'string' },
			});
		} finally {
			server.stop();
		}
	});
});
