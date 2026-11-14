// The app is a single-page client: it talks to the game server over a WebSocket and
// draws the map with WASM, so nothing here is rendered on a server.
export const ssr = false;
export const prerender = false;
