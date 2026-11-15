// Encoding and decoding of game-socket frames. Every frame is one binary
// `ClientMessage` (up) or `ServerMessage` (down). The types come from the
// generated contracts; nothing that crosses the socket is hand-written.
import { create, fromBinary, toBinary } from '@bufbuild/protobuf';
import {
	ClientMessageSchema,
	HelloSchema,
	IntentSchema,
	MediaTokenRequestSchema,
	ServerMessageSchema,
	type Intent,
	type ServerMessage
} from '@tomeandtile/contracts';

/** What an intent asks for: one case of `Intent.kind`, with its message. */
export type IntentKind = Exclude<Intent['kind'], { case: undefined }>;

export function encodeHello(campaignId: string, sessionToken: string, lastSeq = 0n): Uint8Array {
	const hello = create(HelloSchema, { campaignId, sessionToken, lastSeq });
	return toBinary(
		ClientMessageSchema,
		create(ClientMessageSchema, { kind: { case: 'hello', value: hello } })
	);
}

export function encodeIntent(clientIntentId: string, kind: IntentKind): Uint8Array {
	const intent = create(IntentSchema, { clientIntentId, kind });
	return toBinary(
		ClientMessageSchema,
		create(ClientMessageSchema, { kind: { case: 'intent', value: intent } })
	);
}

export function encodeMediaTokenRequest(): Uint8Array {
	const request = create(MediaTokenRequestSchema, {});
	return toBinary(
		ClientMessageSchema,
		create(ClientMessageSchema, { kind: { case: 'mediaTokenRequest', value: request } })
	);
}

/** Decode a frame from the server, or return null if it is not a valid `ServerMessage`. */
export function decodeServerMessage(bytes: Uint8Array): ServerMessage | null {
	try {
		return fromBinary(ServerMessageSchema, bytes);
	} catch {
		return null;
	}
}
