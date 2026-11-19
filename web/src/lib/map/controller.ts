// Wires the map renderer (crates/renderer, a WASM module) to the game socket. The
// renderer owns the canvas and everything drawn on it (ADR 0011); this file only
// passes bytes in and out, so it never decodes a message or draws anything.
//
// The controller is plain TypeScript with the renderer, the frame scheduler and the
// socket passed in, so tests run it without a browser or a GPU.

/** The part of the renderer's published WASM interface that web/ uses (crates/renderer/README.md). */
export interface MapRenderer {
	applyServerMessage(bytes: Uint8Array): void;
	setIntentHandler(handler: (bytes: Uint8Array) => void): void;
	setRejectionHandler(handler: (code: string, message: string, clientIntentId: string) => void): void;
	resize(cssWidth: number, cssHeight: number, devicePixelRatio: number): void;
	render(): void;
	destroy(): void;
}

/** The part of the socket client the map needs. */
export interface MapSocket {
	onFrame(handler: (frame: Uint8Array) => void): () => void;
	sendFrame(frame: Uint8Array): boolean;
}

export interface FrameScheduler {
	request(callback: () => void): number;
	cancel(handle: number): void;
}

export interface MapControllerOptions {
	socket: MapSocket;
	/** Starts the renderer on the canvas. Rejects if no graphics backend works. */
	createRenderer: () => Promise<MapRenderer>;
	scheduler: FrameScheduler;
	/** The map's size in CSS pixels, read when it is resized and when the renderer starts. */
	size: () => { width: number; height: number; devicePixelRatio: number };
	/** The renderer is running and drawing. */
	onReady?: () => void;
	/** The renderer could not start, or stopped. The rest of the page keeps working. */
	onUnavailable: (message: string) => void;
	/** The server refused a token move. `code` is the server's stable code. */
	onRejection: (code: string) => void;
	/** One frame could not be used. The map carries on with the next one. */
	onFrameProblem?: (message: string) => void;
}

export interface MapController {
	/** Call when the map's size or the device pixel ratio changes. */
	resize(): void;
	/** Stop drawing, stop listening and destroy the renderer. Safe to call twice. */
	destroy(): void;
}

/** Frames received while the renderer is still starting are kept this long, then dropped. */
const MAX_WAITING_FRAMES = 200;

export const NO_GRAPHICS_MESSAGE =
	'The map cannot start: this browser has no working graphics support (WebGPU or WebGL2). Chat, dice and the other panels still work.';

export function createMapController(options: MapControllerOptions): MapController {
	let renderer: MapRenderer | null = null;
	let destroyed = false;
	let failed = false;
	let frameHandle: number | null = null;
	// Frames that arrive while the GPU is starting. A Snapshot sent right after Hello
	// would otherwise be lost, because the server does not replay it.
	let waiting: Uint8Array[] = [];

	function deliver(frame: Uint8Array): void {
		if (!renderer) return;
		try {
			renderer.applyServerMessage(frame);
		} catch {
			// The renderer rejects frames it cannot read and changes nothing. Report it
			// in plain words; the renderer's own message is not shown.
			options.onFrameProblem?.('The map could not read one update from the server.');
		}
	}

	const unsubscribe = options.socket.onFrame((frame) => {
		if (destroyed || failed) return;
		if (renderer) {
			deliver(frame);
		} else {
			waiting.push(frame);
			if (waiting.length > MAX_WAITING_FRAMES) waiting.shift();
		}
	});

	function stopLoop(): void {
		if (frameHandle !== null) {
			options.scheduler.cancel(frameHandle);
			frameHandle = null;
		}
	}

	function tick(): void {
		frameHandle = null;
		if (destroyed || !renderer) return;
		try {
			renderer.render();
		} catch {
			// A hard GPU error. Stop drawing instead of failing again every frame.
			failed = true;
			stopLoop();
			options.onUnavailable('The map stopped drawing because of a graphics error. Reload the page to try again.');
			return;
		}
		frameHandle = options.scheduler.request(tick);
	}

	function applySize(target: MapRenderer): void {
		const { width, height, devicePixelRatio } = options.size();
		if (width > 0 && height > 0) target.resize(width, height, devicePixelRatio);
	}

	function start(created: MapRenderer): void {
		if (destroyed) {
			created.destroy();
			return;
		}
		renderer = created;
		try {
			created.setIntentHandler((bytes) => {
				options.socket.sendFrame(bytes);
			});
			created.setRejectionHandler((code) => {
				options.onRejection(code);
			});
			applySize(created);
		} catch {
			failed = true;
			waiting = [];
			renderer = null;
			created.destroy();
			options.onUnavailable(NO_GRAPHICS_MESSAGE);
			return;
		}
		const early = waiting;
		waiting = [];
		for (const frame of early) deliver(frame);
		options.onReady?.();
		frameHandle = options.scheduler.request(tick);
	}

	options.createRenderer().then(start, () => {
		waiting = [];
		if (destroyed) return;
		failed = true;
		options.onUnavailable(NO_GRAPHICS_MESSAGE);
	});

	return {
		resize() {
			if (!destroyed && renderer && !failed) applySize(renderer);
		},
		destroy() {
			if (destroyed) return;
			destroyed = true;
			unsubscribe();
			stopLoop();
			waiting = [];
			if (renderer) {
				const r = renderer;
				renderer = null;
				try {
					r.destroy();
				} catch {
					// Nothing useful to do while leaving the page.
				}
			}
		}
	};
}
