// "Set map image": upload the file, then ask the server to put it on the level. Each
// step can fail in its own plain-language way, and nothing changes on screen until the
// server answers with a scene update.
import type { AssetRef } from '@tomeandtile/contracts';
import type { MapTarget } from '../net';
import {
	httpBaseFromSocketUrl,
	uploadAsset,
	uploadFailureText,
	type UploadFailure,
	type UploadFile
} from './upload';

export type SetMapImageFailure = UploadFailure | 'no_level' | 'not_connected';

export type SetMapImageResult =
	| { ok: true; asset: AssetRef; intentId: string }
	| { ok: false; reason: SetMapImageFailure };

export interface SetMapImageOptions {
	file: UploadFile & Blob;
	/** Where the image goes; null until the server has said which scene is active. */
	target: MapTarget | null;
	socketUrl: string;
	token: string;
	/** Sends the intent for this asset. Returns its id, or null if nothing was sent. */
	sendBackground: (target: MapTarget, assetId: string) => string | null;
	fetch?: typeof fetch;
}

export async function setMapImage(options: SetMapImageOptions): Promise<SetMapImageResult> {
	const target = options.target;
	if (!target) return { ok: false, reason: 'no_level' };
	const uploaded = await uploadAsset({
		baseUrl: httpBaseFromSocketUrl(options.socketUrl),
		campaignId: target.campaignId,
		token: options.token,
		file: options.file,
		fetch: options.fetch
	});
	if (!uploaded.ok) return uploaded;
	const intentId = options.sendBackground(target, uploaded.asset.assetId);
	if (intentId === null) return { ok: false, reason: 'not_connected' };
	return { ok: true, asset: uploaded.asset, intentId };
}

const FLOW_MESSAGES: Record<'no_level' | 'not_connected', string> = {
	no_level: 'There is no map to put the image on yet. Wait for the game to load.',
	not_connected:
		'The image was uploaded, but you are not connected to the game. Reconnect and choose it again.'
};

export function setMapImageFailureText(reason: SetMapImageFailure): string {
	return reason === 'no_level' || reason === 'not_connected'
		? FLOW_MESSAGES[reason]
		: uploadFailureText(reason);
}
