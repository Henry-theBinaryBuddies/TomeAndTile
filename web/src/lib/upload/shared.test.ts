import { create } from '@bufbuild/protobuf';
import { writable } from 'svelte/store';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { IntentRejectedSchema } from '@tomeandtile/contracts';

const state = writable<{ mapTarget: unknown }>({ mapTarget: null });
const lastRejection = writable<unknown>(null);
const clearRejection = vi.fn(() => lastRejection.set(null));
const send = vi.fn(() => 'intent-1');
const run = vi.fn();

vi.mock('../net', () => ({
	getGameClient: () => ({ state, lastRejection, clearRejection, send }),
	getUploadSession: () => ({ socketUrl: 'ws://localhost:8787/ws', token: 'gm' })
}));
vi.mock('./flow', async (original) => ({
	...(await original<typeof import('./flow')>()),
	setMapImage: run
}));

const target = (backgroundAssetId = '') => ({
	campaignId: 'camp',
	sceneId: 's',
	levelId: 'l',
	backgroundAssetId
});

describe('the shared upload store', () => {
	beforeEach(() => {
		vi.resetModules();
		state.set({ mapTarget: null });
		lastRejection.set(null);
		clearRejection.mockClear();
		run.mockReset();
	});

	it('is one store for every caller', async () => {
		const { getUploadStore } = await import('./shared');
		expect(getUploadStore()).toBe(getUploadStore());
	});

	it('uploads to the active level with the page\'s socket address and token', async () => {
		const { startMapImageUpload } = await import('./shared');
		state.set({ mapTarget: target() });
		run.mockResolvedValue({ ok: false, reason: 'unauthorized' });
		await startMapImageUpload(new Blob(['x'], { type: 'image/png' }) as never);
		const options = run.mock.calls[0][0];
		expect(options.target).toEqual(target());
		expect(options.socketUrl).toBe('ws://localhost:8787/ws');
		expect(options.token).toBe('gm');
		expect(options.sendBackground(target(), 'asset-1')).toBe('intent-1');
		expect(send).toHaveBeenCalledOnce();
	});

	it('finishes the upload when the server\'s scene update arrives, with no component mounted', async () => {
		const { startMapImageUpload, getUploadStore } = await import('./shared');
		state.set({ mapTarget: target() });
		run.mockResolvedValue({ ok: true, asset: { assetId: 'asset-1' }, intentId: 'intent-1' });
		await startMapImageUpload(new Blob(['x']) as never);
		expect(getUploadStore().current().kind).toBe('waiting');
		state.set({ mapTarget: target('asset-1') });
		expect(getUploadStore().current().kind).toBe('done');
	});

	it('turns the awaited refusal into an error and clears it from the game client', async () => {
		const { startMapImageUpload, getUploadStore } = await import('./shared');
		state.set({ mapTarget: target() });
		run.mockResolvedValue({ ok: true, asset: { assetId: 'asset-1' }, intentId: 'intent-1' });
		await startMapImageUpload(new Blob(['x']) as never);
		lastRejection.set(create(IntentRejectedSchema, { clientIntentId: 'intent-1', code: 'not_permitted' }));
		expect(getUploadStore().current()).toEqual({ kind: 'error', text: 'You are not allowed to do that.' });
		expect(clearRejection).toHaveBeenCalledOnce();
	});

	it('leaves other refusals for the places that sent them', async () => {
		const { getUploadStore } = await import('./shared');
		getUploadStore();
		lastRejection.set(create(IntentRejectedSchema, { clientIntentId: 'chat-1', code: 'empty_chat' }));
		expect(clearRejection).not.toHaveBeenCalled();
	});
});
