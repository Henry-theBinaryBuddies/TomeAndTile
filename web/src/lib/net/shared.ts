// The one game client the page shares. Nothing connects until a component calls
// `getGameClient().connect()`.
import { createGameClient, tokenFromSearch, type GameClient } from './client';

/** Where the dev mock server listens (`cargo run -p tt-mock-server`). */
export const DEFAULT_SOCKET_URL = 'ws://localhost:8787/ws';

let instance: GameClient | null = null;

function socketUrl(): string {
	return import.meta.env.VITE_TT_SOCKET_URL || DEFAULT_SOCKET_URL;
}

function sessionToken(): string {
	return tokenFromSearch(globalThis.location?.search ?? '', import.meta.env.DEV);
}

export function getGameClient(): GameClient {
	if (!instance) {
		instance = createGameClient({ url: socketUrl(), token: sessionToken() });
	}
	return instance;
}

/** The socket's address and sign-in token, which an upload to the same server needs. */
export function getUploadSession(): { socketUrl: string; token: string } {
	return { socketUrl: socketUrl(), token: sessionToken() };
}
