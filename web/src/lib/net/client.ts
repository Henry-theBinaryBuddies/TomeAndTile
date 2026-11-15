// The game-socket client. One instance owns one WebSocket: it joins, folds the
// server's messages into panel state, reconnects with backoff, and sends
// intents. Panels read the stores and call `send`; they never open a socket.
//
// The client never changes game state itself. `state` changes only when the
// server sends a Snapshot or an Event.
import { readonly, writable, get, type Readable } from 'svelte/store';
import type { IntentRejected, MediaToken } from '@tomeandtile/contracts';
import {
	decodeServerMessage,
	encodeHello,
	encodeIntent,
	encodeMediaTokenRequest,
	type IntentKind
} from './codec';
import { emptyState, foldMessage, type PanelState } from './state';

export type ConnectionState = 'connecting' | 'open' | 'closed';

/** The part of `WebSocket` the client uses, so tests can supply a fake. */
export interface SocketLike {
	binaryType: string;
	readonly readyState: number;
	onopen: ((event: unknown) => void) | null;
	onmessage: ((event: { data: unknown }) => void) | null;
	onclose: ((event: unknown) => void) | null;
	onerror: ((event: unknown) => void) | null;
	send(data: Uint8Array): void;
	close(): void;
}

const SOCKET_OPEN = 1;

export interface GameClientOptions {
	/** WebSocket URL of the game server, e.g. `ws://localhost:8787/ws`. */
	url: string;
	/** Session token sent in `Hello`. Empty if there is none. */
	token?: string;
	/** Campaign id sent in `Hello`. Empty lets the server use its only campaign. */
	campaignId?: string;
	/** Creates the socket. Defaults to `new WebSocket(url)`. */
	createSocket?: (url: string) => SocketLike;
	/** Makes the id for each intent. Defaults to a random UUID. */
	newIntentId?: () => string;
	/** Random number in [0, 1) for backoff jitter. */
	random?: () => number;
	/** How long `requestMediaToken` waits for a reply, in milliseconds. */
	mediaTimeoutMs?: number;
}

/** Failure of `requestMediaToken`. `rejection` is set when the server refused it. */
export class MediaRequestError extends Error {
	readonly rejection: IntentRejected | null;
	constructor(message: string, rejection: IntentRejected | null = null) {
		super(message);
		this.name = 'MediaRequestError';
		this.rejection = rejection;
	}
}

const BACKOFF_BASE_MS = 500;
const BACKOFF_MAX_MS = 10_000;

/** Reconnect delay after `attempt` failed tries (0 for the first): doubling, capped, jittered. */
export function backoffDelay(attempt: number, random: () => number): number {
	const capped = Math.min(BACKOFF_MAX_MS, BACKOFF_BASE_MS * 2 ** attempt);
	return Math.round(capped * (0.5 + random() / 2));
}

/** The development token from `?token=` in the page URL. Always empty outside development. */
export function tokenFromSearch(search: string, dev: boolean): string {
	if (!dev) return '';
	return new URLSearchParams(search).get('token') ?? '';
}

function toBytes(data: unknown): Uint8Array | null {
	if (data instanceof ArrayBuffer) return new Uint8Array(data);
	if (ArrayBuffer.isView(data)) {
		return new Uint8Array(data.buffer, data.byteOffset, data.byteLength);
	}
	return null;
}

function defaultIntentId(): string {
	return globalThis.crypto.randomUUID();
}

export interface GameClient {
	/** Whether the socket is connecting, open or closed. */
	readonly connection: Readable<ConnectionState>;
	/** What the panels show. Replaced by each Snapshot, changed only by server messages. */
	readonly state: Readable<PanelState>;
	/** The latest refusal of an intent, or null. */
	readonly lastRejection: Readable<IntentRejected | null>;
	/** Plain-language note about the last frame that could not be used, or null. */
	readonly problem: Readable<string | null>;
	/** Open the socket and keep it open. Does nothing if already running. */
	connect(): void;
	/** Close the socket and stop reconnecting. */
	close(): void;
	/**
	 * Send an intent. Returns its `client_intent_id`, or null if nothing was sent
	 * because the client has not joined.
	 */
	send(kind: IntentKind): string | null;
	/**
	 * Ask for a video join token. Resolves with the next `MediaToken`; rejects with a
	 * `MediaRequestError` if the server refuses, the connection closes or it times out.
	 * While a request is waiting, calling again returns the same promise.
	 */
	requestMediaToken(): Promise<MediaToken>;
	/** Call `handler` with the unmodified bytes of every binary frame. Returns an unsubscribe. */
	onFrame(handler: (frame: Uint8Array) => void): () => void;
	/** Clear `lastRejection`, e.g. when the user starts typing again. */
	clearRejection(): void;
}

interface PendingMedia {
	promise: Promise<MediaToken>;
	resolve: (token: MediaToken) => void;
	reject: (error: MediaRequestError) => void;
	timer: ReturnType<typeof setTimeout>;
}

export function createGameClient(options: GameClientOptions): GameClient {
	const createSocket = options.createSocket ?? ((url) => new WebSocket(url) as unknown as SocketLike);
	const newIntentId = options.newIntentId ?? defaultIntentId;
	const random = options.random ?? Math.random;
	const mediaTimeoutMs = options.mediaTimeoutMs ?? 10_000;

	const connection = writable<ConnectionState>('closed');
	const state = writable<PanelState>(emptyState());
	const lastRejection = writable<IntentRejected | null>(null);
	const problem = writable<string | null>(null);
	const frameHandlers = new Set<(frame: Uint8Array) => void>();

	let socket: SocketLike | null = null;
	let running = false;
	let attempt = 0;
	let retryTimer: ReturnType<typeof setTimeout> | null = null;
	let pendingMedia: PendingMedia | null = null;

	function settleMedia(
		action: (pending: PendingMedia) => void
	): boolean {
		const pending = pendingMedia;
		if (!pending) return false;
		pendingMedia = null;
		clearTimeout(pending.timer);
		action(pending);
		return true;
	}

	function open(): void {
		connection.set('connecting');
		const ws = createSocket(options.url);
		ws.binaryType = 'arraybuffer';
		socket = ws;

		ws.onopen = () => {
			if (socket !== ws) return;
			connection.set('open');
			ws.send(encodeHello(options.campaignId ?? '', options.token ?? '', 0n));
		};
		ws.onmessage = (event) => {
			if (socket === ws) handleFrame(event.data);
		};
		ws.onerror = () => {
			// A close event always follows; reconnecting is handled there.
		};
		ws.onclose = () => {
			if (socket !== ws) return;
			socket = null;
			connection.set('closed');
			settleMedia((p) => p.reject(new MediaRequestError('The connection closed.')));
			if (running) scheduleReconnect();
		};
	}

	function scheduleReconnect(): void {
		const delay = backoffDelay(attempt, random);
		attempt += 1;
		retryTimer = setTimeout(() => {
			retryTimer = null;
			if (running) open();
		}, delay);
	}

	function handleFrame(data: unknown): void {
		const bytes = toBytes(data);
		if (!bytes) {
			problem.set('The server sent a text message, which is ignored.');
			return;
		}
		for (const handler of frameHandlers) {
			try {
				handler(bytes);
			} catch {
				problem.set('A frame handler failed.');
			}
		}
		const message = decodeServerMessage(bytes);
		if (!message) {
			problem.set('The server sent a message that could not be read.');
			return;
		}
		switch (message.kind.case) {
			case 'snapshot':
				attempt = 0;
				state.update((s) => foldMessage(s, message));
				break;
			case 'event':
				state.update((s) => foldMessage(s, message));
				break;
			case 'intentRejected': {
				const rejection = message.kind.value;
				const forMedia =
					rejection.clientIntentId === '' &&
					settleMedia((p) => p.reject(new MediaRequestError('The server refused.', rejection)));
				if (!forMedia) lastRejection.set(rejection);
				break;
			}
			case 'mediaToken': {
				const token = message.kind.value;
				settleMedia((p) => p.resolve(token));
				break;
			}
			default:
				break;
		}
	}

	return {
		connection: readonly(connection),
		state: readonly(state),
		lastRejection: readonly(lastRejection),
		problem: readonly(problem),

		connect() {
			if (running) return;
			running = true;
			attempt = 0;
			open();
		},

		close() {
			running = false;
			if (retryTimer !== null) {
				clearTimeout(retryTimer);
				retryTimer = null;
			}
			const ws = socket;
			socket = null;
			if (ws) ws.close();
			connection.set('closed');
			settleMedia((p) => p.reject(new MediaRequestError('The connection closed.')));
		},

		send(kind) {
			if (!socket || socket.readyState !== SOCKET_OPEN || !get(state).joined) return null;
			const id = newIntentId();
			socket.send(encodeIntent(id, kind));
			return id;
		},

		requestMediaToken() {
			if (pendingMedia) return pendingMedia.promise;
			if (!socket || socket.readyState !== SOCKET_OPEN || !get(state).joined) {
				return Promise.reject(new MediaRequestError('Not connected.'));
			}
			let resolve!: (token: MediaToken) => void;
			let reject!: (error: MediaRequestError) => void;
			const promise = new Promise<MediaToken>((res, rej) => {
				resolve = res;
				reject = rej;
			});
			const timer = setTimeout(() => {
				settleMedia((p) => p.reject(new MediaRequestError('No reply from the server.')));
			}, mediaTimeoutMs);
			pendingMedia = { promise, resolve, reject, timer };
			socket.send(encodeMediaTokenRequest());
			return promise;
		},

		onFrame(handler) {
			frameHandlers.add(handler);
			return () => {
				frameHandlers.delete(handler);
			};
		},

		clearRejection() {
			lastRejection.set(null);
		}
	};
}
