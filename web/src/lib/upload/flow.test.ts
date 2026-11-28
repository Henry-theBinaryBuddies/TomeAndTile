import { describe, expect, it, vi } from 'vitest';
import { create, toBinary } from '@bufbuild/protobuf';
import { AssetRefSchema, SetLevelBackgroundIntentSchema } from '@tomeandtile/contracts';
import { setMapImage, setMapImageFailureText } from './flow';
import { buildSetBackgroundKind } from './intent';
import type { MapTarget } from '../net';

const target: MapTarget = { campaignId: 'camp-1', sceneId: 'scene-1', levelId: 'level-1', backgroundAssetId: '', cellSize: 0, gridOffset: { x: 0, y: 0 } };
const file = () => new File([new Uint8Array([1, 2, 3])], 'map.webp', { type: 'image/webp' });
const okFetch = () =>
	vi.fn(async () => new Response(toBinary(AssetRefSchema, create(AssetRefSchema, { assetId: 'asset-7', mediaType: 'image/webp' })), { status: 200 })) as unknown as typeof fetch;
const options = (over: Partial<Parameters<typeof setMapImage>[0]> = {}) => ({
	file: file(),
	target,
	socketUrl: 'ws://localhost:8787/ws',
	token: 'gm',
	sendBackground: vi.fn(() => 'intent-1') as (t: MapTarget, id: string) => string | null,
	fetch: okFetch(),
	...over
});

describe('setMapImage', () => {
	it('uploads to the target campaign, then sends the intent for the returned asset', async () => {
		const o = options();
		const result = await setMapImage(o);
		expect(result.ok).toBe(true);
		if (result.ok) {
			expect(result.asset.assetId).toBe('asset-7');
			expect(result.intentId).toBe('intent-1');
		}
		expect((o.fetch as unknown as ReturnType<typeof vi.fn>).mock.calls[0][0]).toBe('http://localhost:8787/campaigns/camp-1/assets');
		expect(o.sendBackground).toHaveBeenCalledWith(target, 'asset-7');
	});

	it('does nothing when there is no level to put the image on', async () => {
		const o = options({ target: null });
		expect(await setMapImage(o)).toEqual({ ok: false, reason: 'no_level' });
		expect(o.fetch).not.toHaveBeenCalled();
		expect(o.sendBackground).not.toHaveBeenCalled();
	});

	it('does not send the intent when the upload fails, and passes the reason on', async () => {
		const o = options({ fetch: vi.fn(async () => new Response(JSON.stringify({ code: 'not_permitted', message: 'x' }), { status: 403 })) as unknown as typeof fetch });
		expect(await setMapImage(o)).toEqual({ ok: false, reason: 'not_permitted' });
		expect(o.sendBackground).not.toHaveBeenCalled();
	});

	it('reports that the game is not connected when the intent could not be sent', async () => {
		const o = options({ sendBackground: () => null });
		expect(await setMapImage(o)).toEqual({ ok: false, reason: 'not_connected' });
	});
});

describe('setMapImageFailureText', () => {
	it('covers the two flow failures and defers to the upload text for the rest', () => {
		expect(setMapImageFailureText('no_level')).toMatch(/no map/i);
		expect(setMapImageFailureText('not_connected')).toMatch(/not connected/i);
		expect(setMapImageFailureText('not_permitted')).toBe('Only the GM can set the map image.');
	});
});

describe('buildSetBackgroundKind', () => {
	it('names the scene, the level and the asset, and nothing else', () => {
		const kind = buildSetBackgroundKind(target, 'asset-7');
		expect(kind.case).toBe('setLevelBackground');
		if (kind.case === 'setLevelBackground') {
			expect(kind.value).toEqual(create(SetLevelBackgroundIntentSchema, { sceneId: 'scene-1', levelId: 'level-1', assetId: 'asset-7' }));
		}
	});
});
