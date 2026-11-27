import { describe, expect, it, vi } from 'vitest';
import { decodeServerMessage } from '../net/codec';
import { eventFrame, snapshotFrame, chatKind } from '../net/testing';
import {
	createMapController,
	NO_GRAPHICS_MESSAGE,
	MISSED_UPDATES_MESSAGE,
	type FrameScheduler,
	type MapRenderer,
	type MapSocket
} from './controller';

class FakeRenderer implements MapRenderer {
	applied: Uint8Array[] = [];
	resizes: Array<[number, number, number]> = [];
	renders = 0;
	destroyed = 0;
	intent: ((bytes: Uint8Array) => void) | null = null;
	rejection: ((code: string, message: string, id: string) => void) | null = null;
	throwOnApply = false;
	throwOnRender = false;
	throwOnPicking = false;
	picking: boolean[] = [];
	highlights: string[][] = [];
	pick: ((tokenId: string) => void) | null = null;
	applyServerMessage(bytes: Uint8Array) {
		if (this.throwOnApply) throw new Error('bad bytes: secret server text');
		this.applied.push(bytes);
	}
	setIntentHandler(handler: (bytes: Uint8Array) => void) {
		this.intent = handler;
	}
	setRejectionHandler(handler: (code: string, message: string, id: string) => void) {
		this.rejection = handler;
	}
	setPicking(enabled: boolean) {
		if (this.throwOnPicking) throw new Error('secret renderer text');
		this.picking.push(enabled);
	}
	onPick(callback: (tokenId: string) => void) {
		this.pick = callback;
	}
	setHighlights(tokenIds: string[]) {
		this.highlights.push([...tokenIds]);
	}
	resize(w: number, h: number, dpr: number) {
		this.resizes.push([w, h, dpr]);
	}
	render() {
		if (this.throwOnRender) throw new Error('gpu lost');
		this.renders++;
	}
	destroy() {
		this.destroyed++;
	}
}

class FakeSocket implements MapSocket {
	handlers = new Set<(frame: Uint8Array) => void>();
	sent: Uint8Array[] = [];
	joined = true;
	onFrame(handler: (frame: Uint8Array) => void) {
		this.handlers.add(handler);
		return () => {
			this.handlers.delete(handler);
		};
	}
	sendFrame(frame: Uint8Array) {
		if (!this.joined) return false;
		this.sent.push(frame);
		return true;
	}
	receive(frame: Uint8Array) {
		for (const h of [...this.handlers]) h(frame);
	}
}

class FakeScheduler implements FrameScheduler {
	next = 1;
	pending = new Map<number, () => void>();
	request(callback: () => void) {
		const id = this.next++;
		this.pending.set(id, callback);
		return id;
	}
	cancel(handle: number) {
		this.pending.delete(handle);
	}
	/** Run the one waiting callback, as the browser does on the next frame. */
	step() {
		const [id, cb] = [...this.pending][0] ?? [];
		if (id === undefined || !cb) return false;
		this.pending.delete(id);
		cb();
		return true;
	}
}

function setup(overrides: { create?: () => Promise<MapRenderer> } = {}) {
	const renderer = new FakeRenderer();
	const socket = new FakeSocket();
	const scheduler = new FakeScheduler();
	const events = {
		ready: vi.fn(),
		unavailable: vi.fn(),
		rejection: vi.fn(),
		problem: vi.fn(),
		pick: vi.fn()
	};
	let resolve!: (r: MapRenderer) => void;
	let reject!: (e: unknown) => void;
	const created = new Promise<MapRenderer>((res, rej) => {
		resolve = res;
		reject = rej;
	});
	const controller = createMapController({
		socket,
		createRenderer: overrides.create ?? (() => created),
		scheduler,
		size: () => ({ width: 800, height: 600, devicePixelRatio: 2 }),
		onReady: events.ready,
		onUnavailable: events.unavailable,
		onRejection: events.rejection,
		onFrameProblem: events.problem,
		onPick: events.pick
	});
	return { renderer, socket, scheduler, events, controller, resolve, reject };
}

const flush = () => new Promise((r) => setTimeout(r, 0));
const bytes = (...n: number[]) => new Uint8Array(n);

describe('starting up', () => {
	it('keeps early frames, then hands them to the renderer in order, unchanged', async () => {
		const t = setup();
		const a = bytes(1);
		const b = bytes(2, 2);
		t.socket.receive(a);
		t.socket.receive(b);
		expect(t.renderer.applied).toHaveLength(0);
		t.resolve(t.renderer);
		await flush();
		expect(t.renderer.applied).toHaveLength(2);
		expect(t.renderer.applied[0]).toBe(a);
		expect(t.renderer.applied[1]).toBe(b);
		expect(t.events.ready).toHaveBeenCalledOnce();
	});

	it('passes every later frame to the renderer unchanged', async () => {
		const t = setup();
		t.resolve(t.renderer);
		await flush();
		const frame = bytes(7, 8, 9);
		t.socket.receive(frame);
		expect(t.renderer.applied).toEqual([frame]);
		expect(t.renderer.applied[0]).toBe(frame);
	});

	it('sizes the renderer from the map size and starts drawing', async () => {
		const t = setup();
		t.resolve(t.renderer);
		await flush();
		expect(t.renderer.resizes).toEqual([[800, 600, 2]]);
		expect(t.scheduler.step()).toBe(true);
		expect(t.renderer.renders).toBe(1);
		expect(t.scheduler.step()).toBe(true);
		expect(t.renderer.renders).toBe(2);
	});

	it('does not resize to an empty box', async () => {
		const renderer = new FakeRenderer();
		const controller = createMapController({
			socket: new FakeSocket(),
			createRenderer: () => Promise.resolve(renderer),
			scheduler: new FakeScheduler(),
			size: () => ({ width: 0, height: 0, devicePixelRatio: 1 }),
			onUnavailable: () => {},
			onRejection: () => {}
		});
		await flush();
		controller.resize();
		expect(renderer.resizes).toEqual([]);
	});

	const snapshot = (seq: number) => new Uint8Array(snapshotFrame(BigInt(seq)));
	const event = (seq: number) => new Uint8Array(eventFrame(BigInt(seq), chatKind('u', `m${seq}`)));
	// Which frames the renderer got, by their kind and sequence number, for readable checks.
	const seen = (r: FakeRenderer) =>
		r.applied.map((f) => {
			const m = decodeServerMessage(f);
			return `${m?.kind.case}:${m?.kind.case === 'snapshot' ? m.kind.value.seq : m?.kind.case === 'event' ? m.kind.value.seq : '?'}`;
		});

	it('keeps every waiting frame, in order, while no Snapshot has replaced them', async () => {
		const t = setup();
		// Bytes that are not a valid message, so none of them can be mistaken for a Snapshot.
		const frames = Array.from({ length: 300 }, (_, i) => new Uint8Array([255, 255, 255, 255, 255, 255, 255, 255, 255, 255, i % 256]));
		for (const f of frames) t.socket.receive(f);
		t.resolve(t.renderer);
		await flush();
		expect(t.events.problem).not.toHaveBeenCalled();
		expect(t.renderer.applied).toHaveLength(300);
		expect(t.renderer.applied.every((f, i) => f === frames[i])).toBe(true);
	});

	it('delivers the Snapshot and every later frame when many frames wait (Snapshot at 30 of 250)', async () => {
		const t = setup();
		for (let i = 0; i < 250; i++) t.socket.receive(i === 30 ? snapshot(31) : event(i + 1));
		t.resolve(t.renderer);
		await flush();
		const got = seen(t.renderer);
		expect(got).toHaveLength(220);
		expect(got[0]).toBe('snapshot:31');
		expect(got[1]).toBe('event:32');
		expect(got[219]).toBe('event:250');
		expect(t.events.unavailable).not.toHaveBeenCalled();
		expect(t.events.problem).not.toHaveBeenCalled();
	});

	it('drops what an earlier Snapshot made old, keeping only the newest Snapshot and what follows', async () => {
		const t = setup();
		t.socket.receive(snapshot(1));
		t.socket.receive(event(2));
		t.socket.receive(snapshot(3)); // a reconnect while the map was starting
		t.socket.receive(event(4));
		t.resolve(t.renderer);
		await flush();
		expect(seen(t.renderer)).toEqual(['snapshot:3', 'event:4']);
	});

	it('does not stop the map when too many frames follow the Snapshot: it waits for the next Snapshot', async () => {
		const t = setup();
		t.socket.receive(snapshot(1));
		for (let i = 2; i <= 1003; i++) t.socket.receive(event(i));
		expect(t.events.problem).toHaveBeenCalledTimes(1);
		expect(t.events.problem).toHaveBeenCalledWith(MISSED_UPDATES_MESSAGE);
		t.socket.receive(event(1004)); // nothing to apply it to, so it is dropped quietly
		expect(t.events.problem).toHaveBeenCalledTimes(1);
		// A reconnect brings a full Snapshot, and the map starts from it.
		t.socket.receive(snapshot(2000));
		t.socket.receive(event(2001));
		t.resolve(t.renderer);
		await flush();
		expect(seen(t.renderer)).toEqual(['snapshot:2000', 'event:2001']);
		expect(t.events.unavailable).not.toHaveBeenCalled();
		expect(t.events.ready).toHaveBeenCalledTimes(1);
	});

	it('still takes a reconnect Snapshot after the renderer is running, even if it had to wait before', async () => {
		const t = setup();
		t.socket.receive(snapshot(1));
		for (let i = 2; i <= 1003; i++) t.socket.receive(event(i));
		t.resolve(t.renderer);
		await flush();
		expect(t.renderer.applied).toHaveLength(0);
		t.socket.receive(snapshot(5000));
		expect(seen(t.renderer)).toEqual(['snapshot:5000']);
	});
});

describe('intents and rejections', () => {
	it('sends the renderer\'s intent frames through the socket unchanged', async () => {
		const t = setup();
		t.resolve(t.renderer);
		await flush();
		const frame = bytes(5, 4, 3);
		t.renderer.intent?.(frame);
		expect(t.socket.sent).toHaveLength(1);
		expect(t.socket.sent[0]).toBe(frame);
	});

	it('does not throw when the socket is not joined', async () => {
		const t = setup();
		t.socket.joined = false;
		t.resolve(t.renderer);
		await flush();
		expect(() => t.renderer.intent?.(bytes(1))).not.toThrow();
		expect(t.socket.sent).toHaveLength(0);
	});

	it('reports a rejection by its code only, not the server\'s text', async () => {
		const t = setup();
		t.resolve(t.renderer);
		await flush();
		t.renderer.rejection?.('not_permitted', '<b>server text</b>', 'id-1');
		expect(t.events.rejection).toHaveBeenCalledWith('not_permitted');
	});
});

describe('problems', () => {
	it('says so when no graphics backend works, and ignores frames', async () => {
		const t = setup();
		t.reject(new Error('no adapter'));
		await flush();
		expect(t.events.unavailable).toHaveBeenCalledWith(NO_GRAPHICS_MESSAGE);
		expect(t.events.ready).not.toHaveBeenCalled();
		t.socket.receive(bytes(1));
		expect(t.renderer.applied).toHaveLength(0);
		expect(t.scheduler.pending.size).toBe(0);
	});

	it('a frame the renderer refuses is reported in plain words and does not stop the map', async () => {
		const t = setup();
		t.resolve(t.renderer);
		await flush();
		t.renderer.throwOnApply = true;
		t.socket.receive(bytes(1));
		expect(t.events.problem).toHaveBeenCalledOnce();
		expect(String(t.events.problem.mock.calls[0][0])).not.toContain('secret');
		t.renderer.throwOnApply = false;
		t.socket.receive(bytes(2));
		expect(t.renderer.applied).toHaveLength(1);
		expect(t.scheduler.step()).toBe(true);
		expect(t.renderer.renders).toBe(1);
	});

	it('stops drawing after a hard graphics error', async () => {
		const t = setup();
		t.resolve(t.renderer);
		await flush();
		t.renderer.throwOnRender = true;
		expect(t.scheduler.step()).toBe(true);
		expect(t.events.unavailable).toHaveBeenCalledOnce();
		expect(t.scheduler.pending.size).toBe(0);
		t.socket.receive(bytes(1));
		expect(t.renderer.applied).toHaveLength(0);
	});

	it('a renderer that fails while being set up is destroyed and reported', async () => {
		const t = setup();
		t.renderer.setIntentHandler = () => {
			throw new Error('boom');
		};
		t.resolve(t.renderer);
		await flush();
		expect(t.events.unavailable).toHaveBeenCalledWith(NO_GRAPHICS_MESSAGE);
		expect(t.renderer.destroyed).toBe(1);
		expect(t.scheduler.pending.size).toBe(0);
	});
});

describe('resize and destroy', () => {
	it('resize passes the current size on', async () => {
		const t = setup();
		t.resolve(t.renderer);
		await flush();
		t.controller.resize();
		expect(t.renderer.resizes).toHaveLength(2);
	});

	it('destroy stops everything and destroys the renderer once', async () => {
		const t = setup();
		t.resolve(t.renderer);
		await flush();
		t.controller.destroy();
		t.controller.destroy();
		expect(t.renderer.destroyed).toBe(1);
		expect(t.socket.handlers.size).toBe(0);
		expect(t.scheduler.pending.size).toBe(0);
		t.controller.resize();
		expect(t.renderer.resizes).toHaveLength(1);
	});

	it('destroying before the renderer is ready destroys it when it arrives', async () => {
		const t = setup();
		t.socket.receive(bytes(1));
		t.controller.destroy();
		expect(t.socket.handlers.size).toBe(0);
		t.resolve(t.renderer);
		await flush();
		expect(t.renderer.destroyed).toBe(1);
		expect(t.renderer.applied).toHaveLength(0);
		expect(t.events.ready).not.toHaveBeenCalled();
		expect(t.scheduler.pending.size).toBe(0);
	});

	it('destroying before a failed start reports nothing', async () => {
		const t = setup();
		t.controller.destroy();
		t.reject(new Error('no adapter'));
		await flush();
		expect(t.events.unavailable).not.toHaveBeenCalled();
	});
});

describe('picking targets', () => {
	it('starts with picking off and nothing highlighted', async () => {
		const t = setup();
		t.resolve(t.renderer);
		await flush();
		expect(t.renderer.picking).toEqual([false]);
		expect(t.renderer.highlights).toEqual([[]]);
	});

	it('remembers settings made while the renderer starts and applies the latest on start', async () => {
		const t = setup();
		t.controller.setPicking(true);
		t.controller.setHighlights(['a']);
		t.controller.setHighlights(['a', 'b']);
		t.resolve(t.renderer);
		await flush();
		expect(t.renderer.picking.at(-1)).toBe(true);
		expect(t.renderer.highlights.at(-1)).toEqual(['a', 'b']);
	});

	it('passes later changes to the renderer, copying the id list', async () => {
		const t = setup();
		t.resolve(t.renderer);
		await flush();
		const ids = ['x'];
		t.controller.setPicking(true);
		t.controller.setHighlights(ids);
		ids.push('changed later');
		expect(t.renderer.picking.at(-1)).toBe(true);
		expect(t.renderer.highlights.at(-1)).toEqual(['x']);
		t.controller.setPicking(false);
		t.controller.setHighlights([]);
		expect(t.renderer.picking.at(-1)).toBe(false);
		expect(t.renderer.highlights.at(-1)).toEqual([]);
	});

	it('reports a picked token by its id only', async () => {
		const t = setup();
		t.resolve(t.renderer);
		await flush();
		t.renderer.pick?.('token-aria');
		expect(t.events.pick).toHaveBeenCalledWith('token-aria');
	});

	it('does nothing, and does not throw, when the map is not available', async () => {
		const t = setup();
		t.reject(new Error('no gpu'));
		await flush();
		expect(() => {
			t.controller.setPicking(true);
			t.controller.setHighlights(['a']);
		}).not.toThrow();
		expect(t.renderer.picking).toEqual([]);
	});

	it('ignores changes after destroy', async () => {
		const t = setup();
		t.resolve(t.renderer);
		await flush();
		t.controller.destroy();
		const before = t.renderer.picking.length;
		t.controller.setPicking(true);
		expect(t.renderer.picking).toHaveLength(before);
	});

	it('reports a renderer that refuses the settings in plain words and keeps running', async () => {
		const t = setup();
		t.resolve(t.renderer);
		await flush();
		t.renderer.throwOnPicking = true;
		t.controller.setPicking(true);
		expect(t.events.problem).toHaveBeenCalledWith('The map could not update the picked targets.');
		expect(JSON.stringify(t.events.problem.mock.calls)).not.toContain('secret');
		expect(t.events.unavailable).not.toHaveBeenCalled();
	});
});
