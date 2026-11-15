import { create, fromBinary, toBinary } from '@bufbuild/protobuf';
import { describe, expect, it } from 'vitest';
import {
	ChatIntentSchema,
	ClientMessageSchema,
	RollDiceIntentSchema,
	RollMode,
	ServerMessageSchema
} from '@tomeandtile/contracts';
import { decodeServerMessage, encodeHello, encodeIntent, encodeMediaTokenRequest } from './codec';
import { snapshotMessage } from './testing';

describe('encoding', () => {
	it('encodes Hello', () => {
		const msg = fromBinary(ClientMessageSchema, encodeHello('camp', 'tok', 7n));
		expect(msg.kind.case).toBe('hello');
		if (msg.kind.case !== 'hello') return;
		expect(msg.kind.value.campaignId).toBe('camp');
		expect(msg.kind.value.sessionToken).toBe('tok');
		expect(msg.kind.value.lastSeq).toBe(7n);
	});

	it('encodes a chat intent with its id', () => {
		const kind = {
			case: 'chat' as const,
			value: create(ChatIntentSchema, { text: 'hi' })
		};
		const msg = fromBinary(ClientMessageSchema, encodeIntent('id-1', kind));
		expect(msg.kind.case).toBe('intent');
		if (msg.kind.case !== 'intent') return;
		expect(msg.kind.value.clientIntentId).toBe('id-1');
		expect(msg.kind.value.kind.case).toBe('chat');
	});

	it('encodes a roll intent', () => {
		const kind = {
			case: 'rollDice' as const,
			value: create(RollDiceIntentSchema, { notation: 'd20+5', mode: RollMode.ADVANTAGE })
		};
		const msg = fromBinary(ClientMessageSchema, encodeIntent('id-2', kind));
		if (msg.kind.case !== 'intent' || msg.kind.value.kind.case !== 'rollDice') {
			throw new Error('wrong message');
		}
		expect(msg.kind.value.kind.value.notation).toBe('d20+5');
		expect(msg.kind.value.kind.value.mode).toBe(RollMode.ADVANTAGE);
	});

	it('encodes a media token request', () => {
		const msg = fromBinary(ClientMessageSchema, encodeMediaTokenRequest());
		expect(msg.kind.case).toBe('mediaTokenRequest');
	});
});

describe('decodeServerMessage', () => {
	it('decodes a valid frame', () => {
		const bytes = toBinary(ServerMessageSchema, snapshotMessage(9n));
		expect(decodeServerMessage(bytes)?.kind.case).toBe('snapshot');
	});

	it('returns null for bytes that are not a message', () => {
		expect(decodeServerMessage(new Uint8Array([0xff, 0xff, 0xff, 0xff, 0xff]))).toBeNull();
	});

	it('returns null for a truncated message', () => {
		const bytes = toBinary(ServerMessageSchema, snapshotMessage(9n));
		expect(decodeServerMessage(bytes.slice(0, 3))).toBeNull();
	});
});
