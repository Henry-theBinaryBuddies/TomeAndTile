import { describe, expect, it, vi } from 'vitest';
import { create, toBinary } from '@bufbuild/protobuf';
import { AssetRefSchema } from '@tomeandtile/contracts';
import {
	ACCEPTED_TYPES,
	checkFile,
	httpBaseFromSocketUrl,
	MAX_UPLOAD_BYTES,
	uploadAsset,
	uploadFailureText,
} from './upload';

const png = (bytes = [137, 80, 78, 71]) => new File([new Uint8Array(bytes)], 'map.png', { type: 'image/png' });

function assetBytes(assetId = 'abc123') {
	const asset = create(AssetRefSchema, {
		assetId,
		mediaType: 'image/png',
		url: '/assets/abc123.png',
		width: 1400,
		height: 1000
	});
	return toBinary(AssetRefSchema, asset);
}

function answer(status: number, body: Uint8Array | string = ''): typeof fetch {
	return vi.fn(async () => new Response(body as BodyInit, { status })) as unknown as typeof fetch;
}

const base = { baseUrl: 'http://localhost:8787', campaignId: 'camp-1', token: 'gm' };

describe('httpBaseFromSocketUrl', () => {
	it('turns ws into http and wss into https, and drops the path', () => {
		expect(httpBaseFromSocketUrl('ws://localhost:8787/ws')).toBe('http://localhost:8787');
		expect(httpBaseFromSocketUrl('wss://play.example.com/ws')).toBe('https://play.example.com');
		expect(httpBaseFromSocketUrl('ws://127.0.0.1:8788/ws?x=1')).toBe('http://127.0.0.1:8788');
	});
});

describe('checkFile', () => {
	it('accepts PNG, JPEG and WebP up to the limit', () => {
		for (const type of ACCEPTED_TYPES) expect(checkFile({ size: 1000, type })).toBeNull();
		expect(checkFile({ size: MAX_UPLOAD_BYTES, type: 'image/png' })).toBeNull();
	});

	it('refuses an empty file, one over 20 MiB and other types', () => {
		expect(checkFile({ size: 0, type: 'image/png' })).toBe('empty_file');
		expect(checkFile({ size: MAX_UPLOAD_BYTES + 1, type: 'image/png' })).toBe('file_too_large');
		expect(checkFile({ size: 10, type: 'image/gif' })).toBe('wrong_type');
		expect(checkFile({ size: 10, type: 'application/pdf' })).toBe('wrong_type');
		expect(checkFile({ size: 10, type: '' })).toBe('wrong_type');
	});
});

describe('uploadAsset', () => {
	it('posts the raw file with the bearer token and returns the decoded asset', async () => {
		const fetchFn = answer(200, assetBytes());
		const file = png();
		const result = await uploadAsset({ ...base, file, fetch: fetchFn });
		expect(result.ok).toBe(true);
		if (result.ok) {
			expect(result.asset.assetId).toBe('abc123');
			expect(result.asset.width).toBe(1400);
		}
		const [url, init] = (fetchFn as unknown as ReturnType<typeof vi.fn>).mock.calls[0] as [string, RequestInit];
		expect(url).toBe('http://localhost:8787/campaigns/camp-1/assets');
		expect(init.method).toBe('POST');
		expect(init.body).toBe(file);
		expect(init.headers).toEqual({ Authorization: 'Bearer gm', 'Content-Type': 'image/png' });
	});

	it('encodes the campaign id in the address', async () => {
		const fetchFn = answer(200, assetBytes());
		await uploadAsset({ ...base, campaignId: 'a b/c', file: png(), fetch: fetchFn });
		expect((fetchFn as unknown as ReturnType<typeof vi.fn>).mock.calls[0][0]).toBe(
			'http://localhost:8787/campaigns/a%20b%2Fc/assets'
		);
	});

	it('refuses a file that is too big, the wrong type or empty without sending anything', async () => {
		const fetchFn = answer(200, assetBytes());
		const big = { size: MAX_UPLOAD_BYTES + 1, type: 'image/png' } as unknown as File;
		expect(await uploadAsset({ ...base, file: big, fetch: fetchFn })).toEqual({ ok: false, reason: 'file_too_large' });
		const gif = new File([new Uint8Array([1])], 'a.gif', { type: 'image/gif' });
		expect(await uploadAsset({ ...base, file: gif, fetch: fetchFn })).toEqual({ ok: false, reason: 'wrong_type' });
		expect(await uploadAsset({ ...base, file: new File([], 'e.png', { type: 'image/png' }), fetch: fetchFn })).toEqual({
			ok: false,
			reason: 'empty_file'
		});
		expect(fetchFn).not.toHaveBeenCalled();
	});

	const jsonError = (status: number, code: string): typeof fetch =>
		vi.fn(
			async () =>
				new Response(JSON.stringify({ code, message: 'secret server words' }), {
					status,
					headers: { 'Content-Type': 'application/json' }
				})
		) as unknown as typeof fetch;

	it.each([
		[401, 'unauthorized'],
		[403, 'not_permitted'],
		[404, 'unknown_campaign'],
		[400, 'empty_file'],
		[400, 'unknown_format'],
		[400, 'disallowed_format'],
		[400, 'truncated_header'],
		[400, 'invalid_dimensions'],
		[400, 'too_many_pixels'],
		[400, 'bad_body'],
		[408, 'upload_timeout'],
		[413, 'file_too_large'],
		[500, 'storage_error'],
		[503, 'busy'],
		[503, 'uploads_off'],
		[507, 'storage_full']
	])('returns the server\'s code for HTTP %i %s', async (status, code) => {
		const result = await uploadAsset({ ...base, file: png(), fetch: jsonError(status, code) });
		expect(result).toEqual({ ok: false, reason: code });
	});

	it('falls back to the status when the error body is not usable', async () => {
		for (const [body, status] of [
			['<html>Bad Gateway</html>', 502],
			['', 500],
			['{"code": 5}', 400],
			['{"code": "Not A Code!"}', 400],
			['{"code": null}', 403],
			['null', 404]
		] as const) {
			const result = await uploadAsset({ ...base, file: png(), fetch: answer(status, body) });
			expect(result).toEqual({ ok: false, reason: `http_${status}` });
		}
	});

	it('reports a network failure', async () => {
		const fetchFn = vi.fn(async () => {
			throw new TypeError('Failed to fetch');
		}) as unknown as typeof fetch;
		expect(await uploadAsset({ ...base, file: png(), fetch: fetchFn })).toEqual({ ok: false, reason: 'network' });
	});

	it('reports an answer that is not an AssetRef, or has no asset id', async () => {
		expect(await uploadAsset({ ...base, file: png(), fetch: answer(200, new Uint8Array([255, 255, 255])) })).toEqual({
			ok: false,
			reason: 'bad_response'
		});
		expect(await uploadAsset({ ...base, file: png(), fetch: answer(200, new Uint8Array()) })).toEqual({
			ok: false,
			reason: 'bad_response'
		});
	});

	it('can be tried again after a failure', async () => {
		const failing = answer(503);
		expect((await uploadAsset({ ...base, file: png(), fetch: failing })).ok).toBe(false);
		expect((await uploadAsset({ ...base, file: png(), fetch: answer(200, assetBytes()) })).ok).toBe(true);
	});
});

describe('uploadFailureText', () => {
	const serverCodes = [
		'empty_file', 'file_too_large', 'wrong_type', 'unknown_format', 'disallowed_format',
		'truncated_header', 'invalid_dimensions', 'too_many_pixels', 'bad_body', 'unauthorized',
		'not_permitted', 'unknown_campaign', 'upload_timeout', 'busy', 'uploads_off',
		'storage_full', 'storage_error', 'method_not_allowed', 'network', 'bad_response'
	];

	it('has its own plain text for every code the servers send', () => {
		const seen = new Set<string>();
		for (const code of serverCodes) {
			const text = uploadFailureText(code);
			expect(text.length).toBeGreaterThan(10);
			expect(text).not.toMatch(/undefined|\[object|http_/);
			if (code.includes('_')) expect(text).not.toContain(code);
			seen.add(text);
		}
		// Different problems are not all lumped under one sentence.
		expect(seen.size).toBeGreaterThanOrEqual(14);
		expect(uploadFailureText('file_too_large')).toContain('20 MiB');
		expect(uploadFailureText('not_permitted')).toBe('Only the GM can set the map image.');
	});

	it('goes by the kind of status for a code it does not know', () => {
		expect(uploadFailureText('http_401')).toBe(uploadFailureText('unauthorized'));
		expect(uploadFailureText('http_403')).toBe(uploadFailureText('not_permitted'));
		expect(uploadFailureText('http_413')).toBe(uploadFailureText('file_too_large'));
		expect(uploadFailureText('http_422')).toMatch(/could not use that file/);
		expect(uploadFailureText('http_502')).toMatch(/problem saving/);
		expect(uploadFailureText('some_new_code')).toBe('The upload failed. Try again.');
	});
});
