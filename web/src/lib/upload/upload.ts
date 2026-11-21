// Sends one image to the server (ADR 0014): a POST of the raw file bytes to
// `{server}/campaigns/{campaign_id}/assets` with the same token the socket uses, and an
// `AssetRef` in the answer. Nothing here decides what a good image is: the server checks
// the file's own bytes. The browser only refuses what it can tell cheaply and early, so
// a 25 MiB file is never sent.
import { fromBinary } from '@bufbuild/protobuf';
import { AssetRefSchema, type AssetRef } from '@tomeandtile/contracts';

/** The server's upload limit (ADR 0014: `tt_asset_pipeline::MAX_FILE_BYTES`). */
export const MAX_UPLOAD_BYTES = 20 * 1024 * 1024;

/** What the picker offers and the browser accepts. The server checks the real format. */
export const ACCEPTED_TYPES = ['image/png', 'image/jpeg', 'image/webp'] as const;

/**
 * Why an upload failed: the server's own `code` (every HTTP error is JSON
 * `{"code","message"}`), or one of `wrong_type`, `network`, `bad_response` and
 * `http_<status>` for what the browser decides or when the server gave no usable code.
 * `empty_file` and `file_too_large` are the server's codes too, used here for the
 * checks the browser makes before sending.
 */
export type UploadFailure = string;

export type UploadResult = { ok: true; asset: AssetRef } | { ok: false; reason: UploadFailure };

/** The picture-sized parts of a `File` that the helper uses, so tests need no real file. */
export interface UploadFile {
	readonly size: number;
	readonly type: string;
}

export interface UploadOptions {
	/** The server's HTTP address, e.g. `http://localhost:8787`. See `httpBaseFromSocketUrl`. */
	baseUrl: string;
	campaignId: string;
	token: string;
	file: UploadFile & Blob;
	/** Defaults to the browser's `fetch`. */
	fetch?: typeof fetch;
	signal?: AbortSignal;
}

/** `ws://host:8787/ws` becomes `http://host:8787`; `wss:` becomes `https:`. */
export function httpBaseFromSocketUrl(socketUrl: string): string {
	const url = new URL(socketUrl);
	url.protocol = url.protocol === 'wss:' ? 'https:' : 'http:';
	return url.origin;
}

/** The failure for a file that cannot be sent at all, or null if it may be tried. */
export function checkFile(file: UploadFile): UploadFailure | null {
	if (file.size === 0) return 'empty_file';
	if (file.size > MAX_UPLOAD_BYTES) return 'file_too_large';
	if (!(ACCEPTED_TYPES as readonly string[]).includes(file.type)) return 'wrong_type';
	return null;
}

const NOT_AN_IMAGE = 'The server only accepts PNG, JPEG and WebP images. Choose a different file.';
const SERVER_TROUBLE = 'The server had a problem saving the file. Try again.';

// Text for each code the servers send (crates/server/src/upload.rs and the pipeline's
// error codes; the mock adds empty_file and storage_full). The server's own message is
// never shown.
const MESSAGES: Record<string, string> = {
	// Checked in the browser first, and sent back by the server as well.
	empty_file: 'That file is empty. Choose a PNG, JPEG or WebP image.',
	file_too_large: 'That file is larger than 20 MiB. Choose a smaller image.',
	wrong_type: NOT_AN_IMAGE,
	// The server looked at the file itself.
	unknown_format: NOT_AN_IMAGE,
	disallowed_format: NOT_AN_IMAGE,
	truncated_header: 'That image file looks damaged or cut short. Choose another file.',
	invalid_dimensions: 'The server could not read the size of that image. Choose another file.',
	too_many_pixels: 'That image has too many pixels. Choose a smaller picture.',
	bad_body: 'The server could not read the upload. Try again.',
	// Who may upload, and where.
	unauthorized: 'The server did not accept your sign-in. Reload the page and try again.',
	not_permitted: 'Only the GM can set the map image.',
	unknown_campaign: 'The server does not know this campaign. Reload the page and try again.',
	// The server is not able to take it right now.
	upload_timeout: 'The upload took too long and was stopped. Check your connection and try again.',
	busy: 'The server is busy with other uploads. Try again in a moment.',
	uploads_off: 'Uploads are turned off on this server.',
	storage_full: 'The server has no room for another new image.',
	storage_error: SERVER_TROUBLE,
	method_not_allowed: 'The page and the server do not agree on how to upload. Reload the page and try again.',
	// Not from the server.
	network: 'Could not reach the server. Check your connection and try again.',
	bad_response: 'The server answered, but not in a way this page understands. Try again.'
};

/** Plain-language text for a failure. The server's own words are never shown. */
export function uploadFailureText(reason: UploadFailure): string {
	const known = MESSAGES[reason];
	if (known) return known;
	// An unknown code, or no code at all: go by the kind of HTTP status, if there was one.
	const status = /^http_(\d{3})$/.exec(reason)?.[1];
	if (status) {
		if (status === '401') return MESSAGES.unauthorized;
		if (status === '403') return MESSAGES.not_permitted;
		if (status === '413') return MESSAGES.file_too_large;
		if (status.startsWith('4')) return 'The server could not use that file. Choose a PNG, JPEG or WebP image under 20 MiB.';
		return SERVER_TROUBLE;
	}
	return 'The upload failed. Try again.';
}

/** The code in the server's JSON error, or `http_<status>` if there is none we can trust. */
async function failureOf(response: Response): Promise<UploadFailure> {
	try {
		const body: unknown = await response.json();
		const code = (body as { code?: unknown } | null)?.code;
		// Only a short, plain code is kept; anything else is not trusted as a key.
		if (typeof code === 'string' && /^[a-z_]{1,40}$/.test(code)) return code;
	} catch {
		// Not JSON (a proxy's error page, say): fall through to the status.
	}
	return `http_${response.status}`;
}

export async function uploadAsset(options: UploadOptions): Promise<UploadResult> {
	const early = checkFile(options.file);
	if (early) return { ok: false, reason: early };

	const doFetch = options.fetch ?? globalThis.fetch.bind(globalThis);
	const url = `${options.baseUrl}/campaigns/${encodeURIComponent(options.campaignId)}/assets`;
	let response: Response;
	try {
		response = await doFetch(url, {
			method: 'POST',
			headers: {
				Authorization: `Bearer ${options.token}`,
				'Content-Type': options.file.type
			},
			body: options.file,
			signal: options.signal
		});
	} catch {
		return { ok: false, reason: 'network' };
	}
	if (!response.ok) return { ok: false, reason: await failureOf(response) };

	try {
		const asset = fromBinary(AssetRefSchema, new Uint8Array(await response.arrayBuffer()));
		if (!asset.assetId) return { ok: false, reason: 'bad_response' };
		return { ok: true, asset };
	} catch {
		return { ok: false, reason: 'bad_response' };
	}
}
