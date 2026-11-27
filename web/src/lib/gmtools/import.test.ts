import { create, toBinary } from '@bufbuild/protobuf';
import { describe, expect, it, vi } from 'vitest';
import { CreatureSchema, ImportResultSchema, WarningSchema } from '@tomeandtile/contracts';
import { MAX_IMPORT_BYTES, checkImportFile, importCreature, importFailureText } from './import';

const file = (text = '{}', name = 'goblin.json') =>
	new File([text], name, { type: 'application/json' });

const reply = (over: Parameters<typeof create<typeof ImportResultSchema>>[1] = {}) =>
	toBinary(
		ImportResultSchema,
		create(ImportResultSchema, {
			creature: create(CreatureSchema, { creatureId: 'c1', name: 'Goblin' }),
			...over
		})
	);
const ok = (bytes: Uint8Array) => vi.fn(async () => new Response(bytes as unknown as BodyInit, { status: 200 })) as unknown as typeof fetch;
const fail = (status: number, body: unknown) =>
	vi.fn(async () => new Response(typeof body === 'string' ? body : JSON.stringify(body), { status })) as unknown as typeof fetch;
const options = (over: Partial<Parameters<typeof importCreature>[0]> = {}) => ({
	baseUrl: 'http://srv:8787',
	campaignId: 'camp 1',
	token: 'secret-token',
	file: file(),
	...over
});

describe('checkImportFile', () => {
	it('refuses an empty file, a big file and a file that is not .json', () => {
		expect(checkImportFile({ size: 0, name: 'a.json' })).toBe('empty_file');
		expect(checkImportFile({ size: MAX_IMPORT_BYTES + 1, name: 'a.json' })).toBe('file_too_large');
		expect(checkImportFile({ size: 10, name: 'a.txt' })).toBe('wrong_type');
	});

	it('accepts a .json file up to the limit, in any letter case', () => {
		expect(checkImportFile({ size: MAX_IMPORT_BYTES, name: 'A.JSON' })).toBeNull();
	});
});

describe('importCreature', () => {
	it('posts the raw file to the campaign imports address with the bearer token', async () => {
		const f = ok(reply());
		await importCreature(options({ fetch: f }));
		const [url, init] = (f as unknown as ReturnType<typeof vi.fn>).mock.calls[0] as [string, RequestInit];
		expect(url).toBe('http://srv:8787/campaigns/camp%201/imports');
		expect(init.method).toBe('POST');
		expect((init.headers as Record<string, string>).Authorization).toBe('Bearer secret-token');
		expect(url).not.toContain('secret-token');
	});

	it('returns the name and the importer notes on success', async () => {
		const bytes = reply({
			replaced: true,
			notes: [
				create(WarningSchema, { code: 'multiattack_text', message: 'Multiattack kept as text.' }),
				create(WarningSchema, { code: 'no_message', message: '' })
			]
		});
		const out = await importCreature(options({ fetch: ok(bytes) }));
		expect(out).toEqual({
			ok: true,
			name: 'Goblin',
			replaced: true,
			notes: ['Multiattack kept as text.', 'no_message']
		});
	});

	it('keeps markup in a note as plain text and cuts a very long one', async () => {
		const bytes = reply({
			notes: [create(WarningSchema, { code: 'x', message: `<img src=x onerror=alert(1)>${'a'.repeat(500)}` })]
		});
		const out = await importCreature(options({ fetch: ok(bytes) }));
		if (!out.ok) throw new Error('expected success');
		expect(out.notes[0].startsWith('<img src=x onerror=alert(1)>')).toBe(true);
		expect(out.notes[0].length).toBe(300);
	});

	it('reports an unnamed creature as Unknown', async () => {
		const out = await importCreature(
			options({ fetch: ok(reply({ creature: create(CreatureSchema, { name: ' ' }) })) })
		);
		expect(out).toMatchObject({ ok: true, name: 'Unknown' });
	});

	it('does not send an empty or oversized file', async () => {
		const f = ok(reply());
		expect(await importCreature(options({ fetch: f, file: file('') }))).toEqual({ ok: false, reason: 'empty_file' });
		const big = new File([new Uint8Array(MAX_IMPORT_BYTES + 1)], 'big.json');
		expect(await importCreature(options({ fetch: f, file: big }))).toEqual({ ok: false, reason: 'file_too_large' });
		expect(f).not.toHaveBeenCalled();
	});

	it('returns the server code from a JSON error', async () => {
		for (const [status, code] of [
			[400, 'bad_creature'],
			[403, 'not_permitted'],
			[413, 'file_too_large'],
			[415, 'format_not_supported_yet'],
			[503, 'busy']
		] as const) {
			const out = await importCreature(options({ fetch: fail(status, { code, message: 'x' }) }));
			expect(out).toEqual({ ok: false, reason: code });
		}
	});

	it('falls back to the status when the error is not usable JSON or the code looks wrong', async () => {
		expect(await importCreature(options({ fetch: fail(502, '<html>bad gateway</html>') }))).toEqual({ ok: false, reason: 'http_502' });
		expect(await importCreature(options({ fetch: fail(400, { code: '<script>', message: '' }) }))).toEqual({ ok: false, reason: 'http_400' });
	});

	it('says network when the request cannot be made, and bad_response for a bad answer', async () => {
		const down = vi.fn(async () => { throw new TypeError('down'); }) as unknown as typeof fetch;
		expect(await importCreature(options({ fetch: down }))).toEqual({ ok: false, reason: 'network' });
		expect(await importCreature(options({ fetch: ok(new Uint8Array([255, 255, 255])) }))).toEqual({ ok: false, reason: 'bad_response' });
		expect(await importCreature(options({ fetch: ok(toBinary(ImportResultSchema, create(ImportResultSchema, {}))) }))).toEqual({ ok: false, reason: 'bad_response' });
	});
});

describe('importFailureText', () => {
	it('has plain words for every code the server sends', () => {
		for (const code of ['empty_file', 'file_too_large', 'wrong_type', 'bad_creature', 'format_not_supported_yet', 'too_many_creatures', 'bad_body', 'unauthorized', 'not_permitted', 'unknown_campaign', 'upload_timeout', 'busy', 'method_not_allowed', 'network', 'bad_response']) {
			const text = importFailureText(code);
			expect(text, code).not.toContain('_');
			expect(text, code).not.toMatch(/failed\. Try again\.$/);
		}
	});

	it('has a fallback for an unknown code and for an unknown status', () => {
		expect(importFailureText('weird_code')).toBe('The import failed. Try again.');
		expect(importFailureText('http_418')).toMatch(/could not use that file/);
		expect(importFailureText('http_500')).toMatch(/problem/);
	});
});
