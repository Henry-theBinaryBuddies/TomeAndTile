// The one game client the page shares. Nothing connects until a component calls
// `getGameClient().connect()`.
import { createGameClient, tokenFromSearch, type GameClient } from './client';

/** Where the dev mock server listens (`cargo run -p tt-mock-server`). */
export const DEFAULT_SOCKET_URL = 'ws://localhost:8787/ws';

let instance: GameClient | null = null;

export function getGameClient(): GameClient {
	if (!instance) {
		instance = createGameClient({
			url: import.meta.env.VITE_TT_SOCKET_URL || DEFAULT_SOCKET_URL,
			token: tokenFromSearch(globalThis.location?.search ?? '', import.meta.env.DEV)
		});
	}
	return instance;
}
