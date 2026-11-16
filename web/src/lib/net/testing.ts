// Helpers for the tests only: a fake WebSocket and builders for server messages.
import { create, fromBinary, toBinary } from '@bufbuild/protobuf';
import {
	ChatPostedSchema,
	ClientMessageSchema,
	DiceRolledSchema,
	DieSchema,
	EncounterEndedSchema,
	EncounterSchema,
	EncounterUpdatedSchema,
	EventSchema,
	EventUndoneSchema,
	IntentRejectedSchema,
	MediaTokenSchema,
	MemberSchema,
	Role,
	RollResultSchema,
	ServerMessageSchema,
	SnapshotSchema,
	TokenRemovedSchema,
	TokenSchema,
	TokenUpsertedSchema,
	SceneUpsertedSchema,
	type ClientMessage,
	type Event
} from '@tomeandtile/contracts';
import type { SocketLike } from './client';

export class FakeSocket implements SocketLike {
	static instances: FakeSocket[] = [];
	binaryType = 'blob';
	readyState = 0;
	onopen: ((event: unknown) => void) | null = null;
	onmessage: ((event: { data: unknown }) => void) | null = null;
	onclose: ((event: unknown) => void) | null = null;
	onerror: ((event: unknown) => void) | null = null;
	sent: Uint8Array[] = [];

	constructor(readonly url: string) {
		FakeSocket.instances.push(this);
	}
	send(data: Uint8Array): void {
		this.sent.push(data);
	}
	close(): void {
		this.readyState = 3;
		this.onclose?.({});
	}
	/** Test side: the connection opens. */
	open(): void {
		this.readyState = 1;
		this.onopen?.({});
	}
	/** Test side: the server sends a frame. */
	receive(data: unknown): void {
		this.onmessage?.({ data });
	}
	/** Test side: the connection drops. */
	drop(): void {
		this.readyState = 3;
		this.onclose?.({});
	}
	sentMessages(): ClientMessage[] {
		return this.sent.map((bytes) => fromBinary(ClientMessageSchema, bytes));
	}
	static reset(): void {
		FakeSocket.instances = [];
	}
	static last(): FakeSocket {
		return FakeSocket.instances[FakeSocket.instances.length - 1];
	}
}

/** Bytes of a `ServerMessage`, as an ArrayBuffer like a real socket delivers. */
function frame(init: Parameters<typeof create<typeof ServerMessageSchema>>[1]): ArrayBuffer {
	const bytes = toBinary(ServerMessageSchema, create(ServerMessageSchema, init));
	return bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength) as ArrayBuffer;
}

export function snapshotMessage(seq = 5n, tokens: [string, string][] = [['t-aria', 'Aria'], ['t-gob', 'Goblin']]) {
	return create(ServerMessageSchema, {
		kind: {
			case: 'snapshot',
			value: create(SnapshotSchema, {
				seq,
				userId: 'mock-player',
				role: Role.PLAYER,
				activeSceneId: 'scene-1',
				members: [
					create(MemberSchema, {
						userId: 'mock-gm',
						displayName: 'Gale',
						role: Role.GM,
						online: true
					}),
					create(MemberSchema, {
						userId: 'mock-player',
						displayName: 'Pip',
						role: Role.PLAYER,
						online: true
					})
				],
				tokens: tokens.map(([tokenId, name]) => create(TokenSchema, { tokenId, name })),
				encounters: [
					create(EncounterSchema, { encounterId: 'enc-1', round: 1, turnIndex: 0 })
				]
			})
		}
	});
}

export function snapshotFrame(seq = 5n): ArrayBuffer {
	return frame({ kind: snapshotMessage(seq).kind });
}

export function eventMessage(seq: bigint, kind: Event['kind']) {
	return create(ServerMessageSchema, {
		kind: { case: 'event', value: create(EventSchema, { seq, atMs: 1000n + seq, kind }) }
	});
}

export function eventFrame(seq: bigint, kind: Event['kind']): ArrayBuffer {
	return frame({ kind: eventMessage(seq, kind).kind });
}

export const chatKind = (userId: string, text: string): Event['kind'] => ({
	case: 'chatPosted',
	value: create(ChatPostedSchema, { userId, text })
});

export const rollKind = (rollId: string, total: number): Event['kind'] => ({
	case: 'diceRolled',
	value: create(DiceRolledSchema, {
		roll: create(RollResultSchema, {
			rollId,
			notation: '1d20+5',
			total,
			dice: [create(DieSchema, { sides: 20, value: total - 5 })]
		})
	})
});

export const encounterUpdatedKind = (id: string, round: number): Event['kind'] => ({
	case: 'encounterUpdated',
	value: create(EncounterUpdatedSchema, {
		encounter: create(EncounterSchema, { encounterId: id, round })
	})
});

export const encounterEndedKind = (id: string): Event['kind'] => ({
	case: 'encounterEnded',
	value: create(EncounterEndedSchema, { encounterId: id })
});

export const undoneKind = (seq: bigint): Event['kind'] => ({
	case: 'eventUndone',
	value: create(EventUndoneSchema, { undoneSeq: seq })
});

export const tokenUpsertedKind = (tokenId: string, name: string): Event['kind'] => ({
	case: 'tokenUpserted',
	value: create(TokenUpsertedSchema, { token: create(TokenSchema, { tokenId, name }) })
});

export const sceneUpsertedKind = (): Event['kind'] => ({
	case: 'sceneUpserted',
	value: create(SceneUpsertedSchema, {})
});

export const tokenRemovedKind = (id: string): Event['kind'] => ({
	case: 'tokenRemoved',
	value: create(TokenRemovedSchema, { tokenId: id })
});

export function rejectedFrame(clientIntentId: string, code: string): ArrayBuffer {
	return frame({
		kind: {
			case: 'intentRejected',
			value: create(IntentRejectedSchema, { clientIntentId, code, message: 'text' })
		}
	});
}

export function mediaTokenFrame(token = 'jwt'): ArrayBuffer {
	return frame({
		kind: {
			case: 'mediaToken',
			value: create(MediaTokenSchema, {
				sfuUrl: 'wss://sfu.example',
				token,
				room: 'tt-campaign-mock',
				expiresAtMs: 99n,
				canPublish: true
			})
		}
	});
}
