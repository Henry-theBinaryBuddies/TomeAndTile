// Sends one creature file to the server (ADR 0015): a POST of the raw file bytes to
// `{server}/campaigns/{campaign_id}/imports` with the same token the socket uses, and an
// `ImportResult` in the answer. The server runs the importer and decides what is stored: the
// browser only refuses what it can tell cheaply, so an empty or huge file is never sent.
// The creature list on screen changes when the server's CreatureUpserted arrives, not from
// this answer.
import { fromBinary } from '@bufbuild/protobuf';
import { ImportResultSchema } from '@tomeandtile/contracts';

/** The server's limit (crates/server `MAX_IMPORT_BYTES`): 1 MiB. */
export const MAX_IMPORT_BYTES = 1024 * 1024;

/** Longest note shown. The server's notes are short; this only guards the page. */
const MAX_NOTE_CHARS = 300;

/** The server's own `code`, or one of `wrong_type`, `network`, `bad_response`, `http_<status>`. */
export type ImportFailure = string;

export interface ImportDone {
	ok: true;
	name: string;
	/** True when the file replaced a creature imported earlier. */
	replaced: boolean;
	/** What the importer could not map. Plain text, never HTML. */
	notes: string[];
}
export type ImportOutcome = ImportDone | { ok: false; reason: ImportFailure };

/** The parts of a `File` the helper uses, so tests need no real file. */
export interface ImportFile {
	readonly size: number;
	readonly name: string;
}

export interface ImportOptions {
	/** The server's HTTP address. See `httpBaseFromSocketUrl` in `../upload`. */
	baseUrl: string;
	campaignId: string;
	token: string;
	file: ImportFile & Blob;
	/** Defaults to the browser's `fetch`. */
	fetch?: typeof fetch;
	signal?: AbortSignal;
}

/** Why a file cannot be sent at all, or null if it may be tried. The server checks again. */
export function checkImportFile(file: ImportFile): ImportFailure | null {
	if (file.size === 0) return 'empty_file';
	if (file.size > MAX_IMPORT_BYTES) return 'file_too_large';
	if (!file.name.toLowerCase().endsWith('.json')) return 'wrong_type';
	return null;
}

const MESSAGES: Record<string, string> = {
	// Checked in the browser first, and sent back by the server as well.
	empty_file: 'That file is empty. Choose a creature file (.json).',
	file_too_large: 'That file is larger than 1 MiB. Choose a smaller file.',
	wrong_type: 'Choose a creature file that ends in .json.',
	// From the server (crates/server/src/import.rs).
	bad_creature: 'The server could not use that file as a creature. Check that it is a creature export.',
	format_not_supported_yet: 'The server cannot read Creature Cauldron files yet.',
	too_many_creatures: 'This campaign has as many creatures as it can hold.',
	bad_body: 'The server could not read the file. Try again.',
	unauthorized: 'The server did not accept your sign-in. Reload the page and try again.',
	not_permitted: 'Only the GM can import creatures.',
	unknown_campaign: 'The server does not know this campaign. Reload the page and try again.',
	upload_timeout: 'The upload took too long and was stopped. Check your connection and try again.',
	busy: 'The server is busy with other imports. Try again in a moment.',
	method_not_allowed: 'The page and the server do not agree on how to import. Reload the page and try again.',
	// Not from the server.
	network: 'Could not reach the server. Check your connection and try again.',
	bad_response: 'The server answered, but not in a way this page understands. Try again.'
};

/** Plain-language text for a failure. The server's own words are never shown. */
export function importFailureText(reason: ImportFailure): string {
	const known = MESSAGES[reason];
	if (known) return known;
	const status = /^http_(\d{3})$/.exec(reason)?.[1];
	if (status === '401') return MESSAGES.unauthorized;
	if (status === '403') return MESSAGES.not_permitted;
	if (status === '413') return MESSAGES.file_too_large;
	if (status === '415') return MESSAGES.format_not_supported_yet;
	if (status?.startsWith('4')) return 'The server could not use that file. Choose a creature file under 1 MiB.';
	if (status) return 'The server had a problem with the import. Try again.';
	return 'The import failed. Try again.';
}

async function failureOf(response: Response): Promise<ImportFailure> {
	try {
		const body: unknown = await response.json();
		const code = (body as { code?: unknown } | null)?.code;
		if (typeof code === 'string' && /^[a-z_]{1,40}$/.test(code)) return code;
	} catch {
		// Not JSON (a proxy's error page, say): fall through to the status.
	}
	return `http_${response.status}`;
}

export async function importCreature(options: ImportOptions): Promise<ImportOutcome> {
	const early = checkImportFile(options.file);
	if (early) return { ok: false, reason: early };

	const doFetch = options.fetch ?? globalThis.fetch.bind(globalThis);
	const url = `${options.baseUrl}/campaigns/${encodeURIComponent(options.campaignId)}/imports`;
	let response: Response;
	try {
		response = await doFetch(url, {
			method: 'POST',
			headers: { Authorization: `Bearer ${options.token}`, 'Content-Type': 'application/json' },
			body: options.file,
			signal: options.signal
		});
	} catch {
		return { ok: false, reason: 'network' };
	}
	if (!response.ok) return { ok: false, reason: await failureOf(response) };

	try {
		const result = fromBinary(ImportResultSchema, new Uint8Array(await response.arrayBuffer()));
		if (!result.creature) return { ok: false, reason: 'bad_response' };
		return {
			ok: true,
			name: result.creature.name.trim() || 'Unknown',
			replaced: result.replaced,
			notes: result.notes
				.map((n) => (n.message.trim() || n.code).slice(0, MAX_NOTE_CHARS))
				.filter((t) => t !== '')
		};
	} catch {
		return { ok: false, reason: 'bad_response' };
	}
}
