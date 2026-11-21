// The one upload store the page shares, wired to the game client the first time the
// control asks for it. It listens for the server's scene update and refusals from then on,
// whether or not the control is on screen.
import type { MapTarget } from '../net';
import { getGameClient, getUploadSession } from '../net';
import { buildSetBackgroundKind } from './intent';
import { createUploadStore, type UploadStore } from './store';
import type { UploadFile } from './upload';

let instance: UploadStore | null = null;

export function getUploadStore(): UploadStore {
	if (!instance) {
		const store = createUploadStore();
		const client = getGameClient();
		client.state.subscribe((s) => store.noteMapTarget(s.mapTarget));
		client.lastRejection.subscribe((rejection) => {
			if (store.noteRejection(rejection)) client.clearRejection();
		});
		instance = store;
	}
	return instance;
}

/** Upload this file as the map image of the active level. False if an upload is already running. */
export function startMapImageUpload(file: UploadFile & Blob): Promise<boolean> {
	const client = getGameClient();
	let target: MapTarget | null = null;
	client.state.subscribe((s) => (target = s.mapTarget))();
	const session = getUploadSession();
	return getUploadStore().start({
		file,
		target,
		socketUrl: session.socketUrl,
		token: session.token,
		sendBackground: (t, assetId) => client.send(buildSetBackgroundKind(t, assetId))
	});
}
