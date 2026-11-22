// The state of the "Set map image" upload, kept outside any component. The control is
// unmounted whenever its panel is collapsed, moved to another dock, or its phone sheet is
// closed, and an upload must not be forgotten (or started twice) when that happens. So the
// status, the asset id, the id of the intent being waited for and the confirm timer all live
// here, in one store the control only reads.
//
// States: idle, uploading (the POST), waiting (the intent was sent; the map changes only when
// the server's scene update arrives), done, error.
import { writable, type Readable } from 'svelte/store';
import type { IntentRejected } from '@tomeandtile/contracts';
import type { MapTarget } from '../net';
import { rejectionText } from '../chat';
import { setMapImage, setMapImageFailureText, type SetMapImageOptions } from './flow';

/** How long to wait for the server's scene update before saying it has not come. */
export const CONFIRM_TIMEOUT_MS = 8000;

export const NOT_CONFIRMED_TEXT =
	'The server has not confirmed the change yet. Check the map, or try again.';
export const UPLOAD_CRASHED_TEXT = 'Something went wrong while uploading. Reload the page and try again.';

export type UploadStatus =
	| { kind: 'idle' }
	| { kind: 'uploading' }
	| { kind: 'waiting'; assetId: string; intentId: string }
	| { kind: 'done' }
	/** `assetId` is set for a timeout, so a scene update that arrives late can still clear it. */
	| { kind: 'error'; text: string; assetId?: string };

/** True while a second upload must not start. */
export function isBusy(status: UploadStatus): boolean {
	return status.kind === 'uploading' || status.kind === 'waiting';
}

export type StartOptions = Omit<SetMapImageOptions, 'fetch'> & { fetch?: typeof fetch };

export interface UploadStoreDeps {
	run?: typeof setMapImage;
	rejectionText?: (code: string) => string;
	confirmTimeoutMs?: number;
}

export interface UploadStore {
	readonly status: Readable<UploadStatus>;
	/** Current status, for code that is not a component. */
	current(): UploadStatus;
	/** Upload and ask for the background. Returns false, doing nothing, if one is already running. */
	start(options: StartOptions): Promise<boolean>;
	/** Tell the store what the active level's background is now. */
	noteMapTarget(target: MapTarget | null): void;
	/** Tell the store about a refusal. Returns true if it was the one being waited for. */
	noteRejection(rejection: IntentRejected | null): boolean;
}

export function createUploadStore(deps: UploadStoreDeps = {}): UploadStore {
	const run = deps.run ?? setMapImage;
	const textFor = deps.rejectionText ?? rejectionText;
	const confirmTimeoutMs = deps.confirmTimeoutMs ?? CONFIRM_TIMEOUT_MS;

	const store = writable<UploadStatus>({ kind: 'idle' });
	let status: UploadStatus = { kind: 'idle' };
	let timer: ReturnType<typeof setTimeout> | null = null;
	let lastBackground = '';

	function set(next: UploadStatus): void {
		status = next;
		store.set(next);
	}

	function stopTimer(): void {
		if (timer) clearTimeout(timer);
		timer = null;
	}

	function fail(text: string, assetId?: string): void {
		stopTimer();
		set(assetId === undefined ? { kind: 'error', text } : { kind: 'error', text, assetId });
	}

	return {
		status: { subscribe: store.subscribe },
		current: () => status,

		async start(options) {
			if (isBusy(status)) return false;
			stopTimer();
			set({ kind: 'uploading' });
			let result: Awaited<ReturnType<typeof setMapImage>>;
			try {
				result = await run(options);
			} catch {
				// A setup mistake, such as a socket address that is not a full URL. Never leave
				// the control stuck on "uploading".
				fail(UPLOAD_CRASHED_TEXT);
				return true;
			}
			if (!result.ok) {
				fail(setMapImageFailureText(result.reason));
				return true;
			}
			const { assetId } = result.asset;
			if (lastBackground === assetId) {
				set({ kind: 'done' });
				return true;
			}
			set({ kind: 'waiting', assetId, intentId: result.intentId });
			timer = setTimeout(() => {
				timer = null;
				if (status.kind === 'waiting') fail(NOT_CONFIRMED_TEXT, status.assetId);
			}, confirmTimeoutMs);
			return true;
		},

		noteMapTarget(target) {
			lastBackground = target?.backgroundAssetId ?? '';
			if (lastBackground === '') return;
			if (status.kind === 'waiting' && status.assetId === lastBackground) {
				stopTimer();
				set({ kind: 'done' });
			} else if (status.kind === 'error' && status.assetId === lastBackground) {
				// The scene update came after the "not confirmed" message: it did work.
				set({ kind: 'done' });
			}
		},

		noteRejection(rejection) {
			if (!rejection || status.kind !== 'waiting' || rejection.clientIntentId !== status.intentId) {
				return false;
			}
			fail(textFor(rejection.code));
			return true;
		}
	};
}
