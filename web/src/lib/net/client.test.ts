import { create } from '@bufbuild/protobuf';
import { get } from 'svelte/store';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { ChatIntentSchema, Role, type IntentRejected } from '@tomeandtile/contracts';
import { backoffDelay, createGameClient, MediaRequestError, tokenFromSearch } from './client';
import {
	chatKind,
	eventFrame,
	FakeSocket,
	mediaTokenFrame,
	rejectedFrame,
	snapshotFrame
} from './testing';

const chat = (text: string) => ({ case: 'chat' as const, value: create(ChatIntentSchema, { text }) });

function make(extra: Partial<Parameters<typeof createGameClient>[0]> = {}) {
	let n = 0;
	return createGameClient({
		url: 'ws://test/ws',
		token: 'gm',
		campaignId: 'camp',
		createSocket: (url) => new FakeSocket(url),
		newIntentId: () => `intent-${++n}`,
		random: () => 1,
		...extra
	});
}

/** Connect, open and deliver a Snapshot. */
function join(client = make()) {
	client.connect();
	FakeSocket.last().open();
	FakeSocket.last().receive(snapshotFrame());
	return client;
}

beforeEach(() => {
	FakeSocket.reset();
	vi.useFakeTimers();
});
afterEach(() => {
	vi.useRealTimers();
});

describe('connecting', () => {
	it('is closed until connect, then connecting, then open', () => {
		const client = make();
		expect(get(client.connection)).toBe('closed');
		client.connect();
		expect(get(client.connection)).toBe('connecting');
		FakeSocket.last().open();
		expect(get(client.connection)).toBe('open');
	});

	it('asks for binary frames and uses the given URL', () => {
		make().connect();
		expect(FakeSocket.last().url).toBe('ws://test/ws');
		expect(FakeSocket.last().binaryType).toBe('arraybuffer');
	});

	it('sends Hello as the first frame, with the token and campaign', () => {
		const client = make();
		client.connect();
		FakeSocket.last().open();
		const [first] = FakeSocket.last().sentMessages();
		expect(first.kind.case).toBe('hello');
		if (first.kind.case !== 'hello') return;
		expect(first.kind.value.sessionToken).toBe('gm');
		expect(first.kind.value.campaignId).toBe('camp');
		expect(FakeSocket.last().sent).toHaveLength(1);
	});

	it('connect twice opens one socket', () => {
		const client = make();
		client.connect();
		client.connect();
		expect(FakeSocket.instances).toHaveLength(1);
	});

	it('is not joined until the Snapshot arrives', () => {
		const client = make();
		client.connect();
		FakeSocket.last().open();
		expect(get(client.state).joined).toBe(false);
		FakeSocket.last().receive(snapshotFrame());
		const s = get(client.state);
		expect(s.joined).toBe(true);
		expect(s.role).toBe(Role.PLAYER);
		expect(s.members).toHaveLength(2);
	});
});

describe('state from the server', () => {
	it('applies events after the Snapshot', () => {
		const client = join();
		FakeSocket.last().receive(eventFrame(6n, chatKind('mock-gm', 'hi')));
		expect(get(client.state).log).toHaveLength(1);
	});
});

describe('send', () => {
	it('sends the frame and returns a fresh id each time', () => {
		const client = join();
		const a = client.send(chat('one'));
		const b = client.send(chat('two'));
		expect(a).toBe('intent-1');
		expect(b).toBe('intent-2');
		const sent = FakeSocket.last().sentMessages();
		expect(sent).toHaveLength(3); // Hello plus two intents
		const ids = sent.slice(1).map((m) => (m.kind.case === 'intent' ? m.kind.value.clientIntentId : ''));
		expect(ids).toEqual(['intent-1', 'intent-2']);
	});

	it('sends nothing and returns null before joining', () => {
		const client = make();
		expect(client.send(chat('x'))).toBeNull();
		client.connect();
		FakeSocket.last().open();
		expect(client.send(chat('x'))).toBeNull();
		expect(FakeSocket.last().sent).toHaveLength(1); // only Hello
	});

	it('sendFrame sends the given bytes unchanged once joined', () => {
		const client = join();
		const frame = new Uint8Array([1, 2, 3, 250]);
		expect(client.sendFrame(frame)).toBe(true);
		const sent = FakeSocket.last().sent;
		expect(sent).toHaveLength(2); // Hello plus the frame
		expect(sent[1]).toBe(frame);
	});

	it('sendFrame sends nothing and returns false before joining or after close', () => {
		const client = make();
		const frame = new Uint8Array([9]);
		expect(client.sendFrame(frame)).toBe(false);
		client.connect();
		FakeSocket.last().open();
		expect(client.sendFrame(frame)).toBe(false);
		FakeSocket.last().receive(snapshotFrame());
		client.close();
		expect(client.sendFrame(frame)).toBe(false);
		expect(FakeSocket.last().sent).toHaveLength(1); // only Hello
	});

	it('uses a random UUID by default', () => {
		const client = join(
			createGameClient({
				url: 'ws://test/ws',
				createSocket: (url) => new FakeSocket(url)
			})
		);
		const id = client.send(chat('x'));
		expect(id).toMatch(/^[0-9a-f-]{36}$/);
	});
});

describe('rejections', () => {
	it('are exposed and can be cleared', () => {
		const client = join();
		const id = client.send(chat(''))!;
		FakeSocket.last().receive(rejectedFrame(id, 'empty_chat'));
		const rejection = get(client.lastRejection) as IntentRejected;
		expect(rejection.code).toBe('empty_chat');
		expect(rejection.clientIntentId).toBe(id);
		client.clearRejection();
		expect(get(client.lastRejection)).toBeNull();
	});
});

describe('reconnecting', () => {
	it('backs off, reconnects and replaces state with the new Snapshot', () => {
		const client = join();
		FakeSocket.last().receive(eventFrame(6n, chatKind('u', 'old chat')));
		expect(get(client.state).log).toHaveLength(1);

		FakeSocket.last().drop();
		expect(get(client.connection)).toBe('closed');
		expect(FakeSocket.instances).toHaveLength(1);

		vi.advanceTimersByTime(499);
		expect(FakeSocket.instances).toHaveLength(1);
		vi.advanceTimersByTime(1);
		expect(FakeSocket.instances).toHaveLength(2);
		expect(get(client.connection)).toBe('connecting');

		FakeSocket.last().open();
		FakeSocket.last().receive(snapshotFrame(40n));
		const s = get(client.state);
		expect(s.seq).toBe(40n);
		expect(s.log).toEqual([]);
		expect(get(client.connection)).toBe('open');
	});

	it('waits longer after each failed try, and starts over after a Snapshot', () => {
		const client = make();
		client.connect();
		FakeSocket.last().drop(); // never opened: try 1 failed
		vi.advanceTimersByTime(500);
		expect(FakeSocket.instances).toHaveLength(2);
		FakeSocket.last().drop();
		vi.advanceTimersByTime(999);
		expect(FakeSocket.instances).toHaveLength(2);
		vi.advanceTimersByTime(1);
		expect(FakeSocket.instances).toHaveLength(3);

		FakeSocket.last().open();
		FakeSocket.last().receive(snapshotFrame());
		FakeSocket.last().drop();
		vi.advanceTimersByTime(500);
		expect(FakeSocket.instances).toHaveLength(4);
	});

	it('does not reconnect after close()', () => {
		const client = join();
		client.close();
		expect(get(client.connection)).toBe('closed');
		vi.advanceTimersByTime(60_000);
		expect(FakeSocket.instances).toHaveLength(1);
	});

	it('close() cancels a pending reconnect', () => {
		const client = join();
		FakeSocket.last().drop();
		client.close();
		vi.advanceTimersByTime(60_000);
		expect(FakeSocket.instances).toHaveLength(1);
	});

	it('ignores frames from a socket it has replaced', () => {
		const client = join();
		const old = FakeSocket.last();
		old.drop();
		vi.advanceTimersByTime(500);
		FakeSocket.last().open();
		old.receive(snapshotFrame(99n));
		expect(get(client.state).seq).toBe(5n);
	});
});

describe('backoffDelay', () => {
	it('doubles from 500 ms and caps at 10 s', () => {
		const delays = [0, 1, 2, 3, 4, 5, 6].map((n) => backoffDelay(n, () => 1));
		expect(delays).toEqual([500, 1000, 2000, 4000, 8000, 10000, 10000]);
	});

	it('jitters between half and the full delay', () => {
		expect(backoffDelay(2, () => 0)).toBe(1000);
		expect(backoffDelay(2, () => 0.5)).toBe(1500);
	});
});

describe('requestMediaToken', () => {
	it('sends a request and resolves with the next MediaToken', async () => {
		const client = join();
		const promise = client.requestMediaToken();
		const last = FakeSocket.last().sentMessages().at(-1)!;
		expect(last.kind.case).toBe('mediaTokenRequest');
		FakeSocket.last().receive(mediaTokenFrame('abc'));
		const token = await promise;
		expect(token.token).toBe('abc');
		expect(token.room).toBe('tt-campaign-mock');
	});

	it('keeps one request in flight: a second call shares the first', async () => {
		const client = join();
		const a = client.requestMediaToken();
		const b = client.requestMediaToken();
		expect(b).toBe(a);
		expect(FakeSocket.last().sent).toHaveLength(2); // Hello and one request
		FakeSocket.last().receive(mediaTokenFrame());
		await a;
		client.requestMediaToken().catch(() => {});
		expect(FakeSocket.last().sent).toHaveLength(3);
	});

	it('rejects on an IntentRejected with an empty id, without setting lastRejection', async () => {
		const client = join();
		const promise = client.requestMediaToken();
		FakeSocket.last().receive(rejectedFrame('', 'media_not_configured'));
		const error = await promise.catch((e) => e);
		expect(error).toBeInstanceOf(MediaRequestError);
		expect(error.rejection.code).toBe('media_not_configured');
		expect(get(client.lastRejection)).toBeNull();
	});

	it('does not take a rejection that has an intent id', async () => {
		const client = join();
		const promise = client.requestMediaToken();
		FakeSocket.last().receive(rejectedFrame('intent-9', 'empty_chat'));
		expect((get(client.lastRejection) as IntentRejected).code).toBe('empty_chat');
		FakeSocket.last().receive(mediaTokenFrame());
		await expect(promise).resolves.toBeDefined();
	});

	it('rejects when the connection closes', async () => {
		const client = join();
		const promise = client.requestMediaToken();
		FakeSocket.last().drop();
		await expect(promise).rejects.toBeInstanceOf(MediaRequestError);
	});

	it('rejects if the server never replies (the mock ignores it)', async () => {
		const client = join(make({ mediaTimeoutMs: 5000 }));
		const promise = client.requestMediaToken();
		const settled = promise.catch((e) => e);
		vi.advanceTimersByTime(5000);
		expect(await settled).toBeInstanceOf(MediaRequestError);
		client.requestMediaToken().catch(() => {});
		expect(FakeSocket.last().sent).toHaveLength(3);
	});

	it('rejects at once when not joined', async () => {
		const client = make();
		await expect(client.requestMediaToken()).rejects.toBeInstanceOf(MediaRequestError);
	});

	it('ignores a MediaToken nobody asked for', () => {
		const client = join();
		FakeSocket.last().receive(mediaTokenFrame());
		expect(get(client.problem)).toBeNull();
	});
});

describe('onFrame', () => {
	it('passes the unmodified bytes of every frame', () => {
		const client = join(make());
		const seen: Uint8Array[] = [];
		client.onFrame((f) => seen.push(f));
		const buf = eventFrame(6n, chatKind('u', 'x'));
		FakeSocket.last().receive(buf);
		expect(seen).toHaveLength(1);
		expect(seen[0]).toEqual(new Uint8Array(buf));
	});

	it('also passes frames that cannot be decoded', () => {
		const client = join();
		const seen: Uint8Array[] = [];
		client.onFrame((f) => seen.push(f));
		FakeSocket.last().receive(new Uint8Array([255, 255, 255, 255, 255]).buffer);
		expect(seen).toHaveLength(1);
	});

	it('stops after the returned function is called', () => {
		const client = join();
		const handler = vi.fn();
		const off = client.onFrame(handler);
		off();
		FakeSocket.last().receive(eventFrame(6n, chatKind('u', 'x')));
		expect(handler).not.toHaveBeenCalled();
	});

	it('a throwing handler does not stop the client or other handlers', () => {
		const client = join();
		const second = vi.fn();
		client.onFrame(() => {
			throw new Error('boom');
		});
		client.onFrame(second);
		FakeSocket.last().receive(eventFrame(6n, chatKind('u', 'x')));
		expect(second).toHaveBeenCalledOnce();
		expect(get(client.state).log).toHaveLength(1);
		expect(get(client.problem)).not.toBeNull();
	});
});

describe('bad frames', () => {
	it('an undecodable frame is ignored and reported without crashing', () => {
		const client = join();
		const before = get(client.state);
		FakeSocket.last().receive(new Uint8Array([255, 255, 255, 255, 255]).buffer);
		expect(get(client.state)).toBe(before);
		expect(get(client.problem)).toMatch(/could not be read/);
		FakeSocket.last().receive(eventFrame(6n, chatKind('u', 'still works')));
		expect(get(client.state).log).toHaveLength(1);
	});

	it('a text frame is ignored and reported', () => {
		const client = join();
		FakeSocket.last().receive('{"hello":"world"}');
		expect(get(client.problem)).toMatch(/text message/);
		expect(get(client.state).joined).toBe(true);
	});

	it('the report is cleared by the next Snapshot, for example after a reconnect', () => {
		const client = join();
		FakeSocket.last().receive('not a message');
		expect(get(client.problem)).not.toBeNull();
		FakeSocket.last().drop();
		vi.advanceTimersByTime(500);
		FakeSocket.last().open();
		FakeSocket.last().receive(snapshotFrame());
		expect(get(client.problem)).toBeNull();
	});

	it('the report never contains text from the server', () => {
		const client = join();
		FakeSocket.last().receive('<script>alert(1)</script>');
		expect(get(client.problem)).not.toMatch(/script/);
	});
});

describe('tokenFromSearch', () => {
	it('reads ?token= in development', () => {
		expect(tokenFromSearch('?token=gm', true)).toBe('gm');
		expect(tokenFromSearch('?a=1&token=dev%3Apip', true)).toBe('dev:pip');
	});
	it('is empty when there is none', () => {
		expect(tokenFromSearch('', true)).toBe('');
		expect(tokenFromSearch('?x=1', true)).toBe('');
	});
	it('is always empty outside development', () => {
		expect(tokenFromSearch('?token=gm', false)).toBe('');
	});
});
