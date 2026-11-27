import { create } from '@bufbuild/protobuf';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { AssetRefSchema, IntentRejectedSchema } from '@tomeandtile/contracts';
import type { MapTarget } from '../net';
import type { SetMapImageResult } from './flow';
import {
	CONFIRM_TIMEOUT_MS,
	NOT_CONFIRMED_TEXT,
	UPLOAD_CRASHED_TEXT,
	createUploadStore,
	isBusy,
	type StartOptions
} from './store';

const target = (backgroundAssetId = ''): MapTarget => ({
	campaignId: 'camp',
	sceneId: 'scene',
	levelId: 'level',
	backgroundAssetId,
	cellSize: 0,
	gridOffset: { x: 0, y: 0 }
});

const options = (): StartOptions => ({
	file: new Blob(['x'], { type: 'image/png' }) as StartOptions['file'],
	target: target(),
	socketUrl: 'ws://localhost:8787/ws',
	token: 'gm',
	sendBackground: () => 'intent-1'
});

const ok = (assetId = 'asset-1', intentId = 'intent-1'): SetMapImageResult => ({
	ok: true,
	asset: create(AssetRefSchema, { assetId }),
	intentId
});

/** A run function the test finishes by hand, so states in between can be looked at. */
function manualRun() {
	let finish!: (r: SetMapImageResult) => void;
	let crash!: (e: unknown) => void;
	const calls: StartOptions[] = [];
	const run = vi.fn(
		(o: StartOptions) =>
			new Promise<SetMapImageResult>((resolve, reject) => {
				calls.push(o);
				finish = resolve;
				crash = reject;
			})
	);
	return { run, calls, finish: (r: SetMapImageResult) => finish(r), crash: (e: unknown) => crash(e) };
}

const refused = (id: string, code = 'not_permitted') =>
	create(IntentRejectedSchema, { clientIntentId: id, code, message: 'server words' });

beforeEach(() => vi.useFakeTimers());
afterEach(() => vi.useRealTimers());

describe('the happy path', () => {
	it('goes idle, uploading, waiting, done', async () => {
		const m = manualRun();
		const store = createUploadStore({ run: m.run });
		expect(store.current()).toEqual({ kind: 'idle' });
		const started = store.start(options());
		expect(store.current()).toEqual({ kind: 'uploading' });
		m.finish(ok('asset-9', 'intent-9'));
		await started;
		expect(store.current()).toEqual({ kind: 'waiting', assetId: 'asset-9', intentId: 'intent-9' });
		store.noteMapTarget(target('asset-9'));
		expect(store.current()).toEqual({ kind: 'done' });
	});

	it('shows each state to a subscriber, in order', async () => {
		const m = manualRun();
		const store = createUploadStore({ run: m.run });
		const seen: string[] = [];
		store.status.subscribe((s) => seen.push(s.kind));
		const started = store.start(options());
		m.finish(ok());
		await started;
		store.noteMapTarget(target('asset-1'));
		expect(seen).toEqual(['idle', 'uploading', 'waiting', 'done']);
	});

	it('ignores a map update for some other image while waiting', async () => {
		const m = manualRun();
		const store = createUploadStore({ run: m.run });
		const started = store.start(options());
		m.finish(ok('asset-1'));
		await started;
		store.noteMapTarget(target('someone-elses'));
		expect(store.current().kind).toBe('waiting');
		store.noteMapTarget(null);
		expect(store.current().kind).toBe('waiting');
	});

	it('is done at once when the level already has that image', async () => {
		const store = createUploadStore({ run: async () => ok('asset-1') });
		store.noteMapTarget(target('asset-1'));
		await store.start(options());
		expect(store.current()).toEqual({ kind: 'done' });
	});

	it('hands the options to the flow unchanged', async () => {
		const m = manualRun();
		const store = createUploadStore({ run: m.run });
		const o = options();
		const started = store.start(o);
		expect(m.calls[0]).toBe(o);
		m.finish(ok());
		await started;
	});
});

describe('a refusal', () => {
	it('turns waiting into an error with our own text for the code, once', async () => {
		const m = manualRun();
		const store = createUploadStore({ run: m.run, rejectionText: (c) => `text for ${c}` });
		const started = store.start(options());
		m.finish(ok('asset-1', 'intent-7'));
		await started;
		expect(store.noteRejection(refused('intent-7', 'unknown_asset'))).toBe(true);
		expect(store.current()).toEqual({ kind: 'error', text: 'text for unknown_asset' });
	});

	it('ignores a refusal for another intent, or when nothing is being waited for', async () => {
		const m = manualRun();
		const store = createUploadStore({ run: m.run });
		expect(store.noteRejection(refused('intent-7'))).toBe(false);
		const started = store.start(options());
		expect(store.noteRejection(refused('intent-7'))).toBe(false); // still uploading
		m.finish(ok('asset-1', 'intent-7'));
		await started;
		expect(store.noteRejection(refused('other'))).toBe(false);
		expect(store.noteRejection(null)).toBe(false);
		expect(store.current().kind).toBe('waiting');
	});

	it('never shows the server\'s own words', async () => {
		const store = createUploadStore({ run: async () => ok('a', 'i') });
		await store.start(options());
		store.noteRejection(refused('i', 'not_permitted'));
		const s = store.current();
		expect(s.kind === 'error' && s.text).not.toContain('server words');
	});

	it('cancels the timeout, so no second error follows', async () => {
		const store = createUploadStore({ run: async () => ok('a', 'i') });
		await store.start(options());
		store.noteRejection(refused('i'));
		const before = store.current();
		vi.advanceTimersByTime(CONFIRM_TIMEOUT_MS * 2);
		expect(store.current()).toBe(before);
	});
});

describe('the confirm timeout', () => {
	it('turns waiting into the not-confirmed error after 8 seconds, not before', async () => {
		const store = createUploadStore({ run: async () => ok() });
		await store.start(options());
		vi.advanceTimersByTime(CONFIRM_TIMEOUT_MS - 1);
		expect(store.current().kind).toBe('waiting');
		vi.advanceTimersByTime(1);
		expect(store.current()).toMatchObject({ kind: 'error', text: NOT_CONFIRMED_TEXT });
	});

	it('is cancelled when the map update arrives in time', async () => {
		const store = createUploadStore({ run: async () => ok('a') });
		await store.start(options());
		store.noteMapTarget(target('a'));
		vi.advanceTimersByTime(CONFIRM_TIMEOUT_MS * 2);
		expect(store.current()).toEqual({ kind: 'done' });
	});

	it('is cleared if the scene update for that image arrives late', async () => {
		const store = createUploadStore({ run: async () => ok('a') });
		await store.start(options());
		vi.advanceTimersByTime(CONFIRM_TIMEOUT_MS);
		expect(store.current().kind).toBe('error');
		store.noteMapTarget(target('other'));
		expect(store.current().kind).toBe('error');
		store.noteMapTarget(target('a'));
		expect(store.current()).toEqual({ kind: 'done' });
	});

	it('is also kept apart per upload: an old timer cannot fail a newer upload', async () => {
		const store = createUploadStore({ run: async () => ok('a', 'i1') });
		await store.start(options());
		store.noteRejection(refused('i1'));
		vi.advanceTimersByTime(CONFIRM_TIMEOUT_MS - 1000);
		await store.start(options());
		vi.advanceTimersByTime(1000);
		expect(store.current().kind).toBe('waiting');
	});
});

describe('failures', () => {
	it('shows the flow\'s plain text when the upload fails', async () => {
		const store = createUploadStore({ run: async () => ({ ok: false, reason: 'file_too_large' }) });
		await store.start(options());
		expect(store.current()).toEqual({ kind: 'error', text: expect.stringContaining('20 MiB') });
	});

	it('says what is missing when there is no level or no connection', async () => {
		const noLevel = createUploadStore({ run: async () => ({ ok: false, reason: 'no_level' }) });
		await noLevel.start(options());
		expect(noLevel.current()).toMatchObject({ kind: 'error', text: expect.stringContaining('no map') });
		const offline = createUploadStore({ run: async () => ({ ok: false, reason: 'not_connected' }) });
		await offline.start(options());
		expect(offline.current()).toMatchObject({ kind: 'error', text: expect.stringContaining('not connected') });
	});

	it('does not stay stuck on uploading if the flow throws', async () => {
		const m = manualRun();
		const store = createUploadStore({ run: m.run });
		const started = store.start(options());
		m.crash(new TypeError('Invalid URL'));
		await started;
		expect(store.current()).toEqual({ kind: 'error', text: UPLOAD_CRASHED_TEXT });
	});

	it('allows another try after an error', async () => {
		const runs: SetMapImageResult[] = [{ ok: false, reason: 'busy' }, ok('a')];
		const store = createUploadStore({ run: async () => runs.shift()! });
		await store.start(options());
		expect(store.current().kind).toBe('error');
		expect(await store.start(options())).toBe(true);
		expect(store.current().kind).toBe('waiting');
	});

	it('allows another upload after one is done', async () => {
		const store = createUploadStore({ run: async () => ok('a') });
		await store.start(options());
		store.noteMapTarget(target('a'));
		expect(isBusy(store.current())).toBe(false);
		expect(await store.start({ ...options() })).toBe(true);
	});
});

describe('one upload at a time', () => {
	it('refuses a second start while uploading, and does not run the flow again', async () => {
		const m = manualRun();
		const store = createUploadStore({ run: m.run });
		const first = store.start(options());
		expect(await store.start(options())).toBe(false);
		expect(m.run).toHaveBeenCalledTimes(1);
		expect(store.current()).toEqual({ kind: 'uploading' });
		m.finish(ok());
		await first;
	});

	it('refuses a second start while waiting for the server', async () => {
		const run = vi.fn(async () => ok('a'));
		const store = createUploadStore({ run });
		await store.start(options());
		expect(await store.start(options())).toBe(false);
		expect(run).toHaveBeenCalledTimes(1);
		expect(store.current().kind).toBe('waiting');
	});

	it('still refuses after the control is unmounted and mounted again (a new subscriber)', async () => {
		const m = manualRun();
		const store = createUploadStore({ run: m.run });
		const unsubscribe = store.status.subscribe(() => {});
		const first = store.start(options());
		unsubscribe(); // the panel is collapsed
		const seen: string[] = [];
		store.status.subscribe((s) => seen.push(s.kind)); // and opened again
		expect(seen).toEqual(['uploading']);
		expect(await store.start(options())).toBe(false);
		m.finish(ok('a', 'i'));
		await first;
		const again: string[] = [];
		store.status.subscribe((s) => again.push(s.kind));
		expect(again).toEqual(['waiting']);
	});

	it('keeps a finished result for a control that comes back later', async () => {
		const store = createUploadStore({ run: async () => ({ ok: false, reason: 'unauthorized' }) });
		await store.start(options());
		const seen: string[] = [];
		store.status.subscribe((s) => seen.push(s.kind));
		expect(seen).toEqual(['error']);
	});

	it('tells busy states from the rest', () => {
		expect(isBusy({ kind: 'uploading' })).toBe(true);
		expect(isBusy({ kind: 'waiting', assetId: 'a', intentId: 'i' })).toBe(true);
		for (const kind of ['idle', 'done'] as const) expect(isBusy({ kind })).toBe(false);
		expect(isBusy({ kind: 'error', text: 'x' })).toBe(false);
	});
});
