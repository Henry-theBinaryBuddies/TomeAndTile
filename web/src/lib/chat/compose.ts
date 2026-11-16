// Builds the intents the chat panel sends. These only describe a request; the
// server decides what happens, rolls the dice and answers with events.
import { create } from '@bufbuild/protobuf';
import {
	ChatIntentSchema,
	RollDiceIntentSchema,
	RollMode,
	RollVisibility,
	type IntentRejected
} from '@tomeandtile/contracts';
import type { IntentKind } from '../net';

/** A chat intent, or null if the text is empty after trimming. */
export function buildChatKind(text: string): IntentKind | null {
	const trimmed = text.trim();
	if (!trimmed) return null;
	return { case: 'chat', value: create(ChatIntentSchema, { text: trimmed }) };
}

export interface RollForm {
	notation: string;
	mode: RollMode;
	label: string;
	/** Ask for a GM-only roll. Ignored unless the sender is the GM. */
	hidden: boolean;
}

/** A roll intent, or null if the notation is empty. */
export function buildRollKind(form: RollForm, isGm: boolean): IntentKind | null {
	const notation = form.notation.trim();
	if (!notation) return null;
	return {
		case: 'rollDice',
		value: create(RollDiceIntentSchema, {
			notation,
			mode: form.mode,
			label: form.label.trim(),
			visibility: isGm && form.hidden ? RollVisibility.GM_ONLY : RollVisibility.PUBLIC
		})
	};
}

/**
 * What the chat box should hold after a rejection. The box is cleared when a
 * message is sent; if the server refuses that message and the box is still
 * empty, the text comes back so nothing the person typed is lost.
 */
export function textAfterRejection(
	sent: { id: string; text: string } | null,
	rejection: IntentRejected | null,
	current: string
): string {
	if (!sent || !rejection || rejection.clientIntentId !== sent.id) return current;
	return current === '' ? sent.text : current;
}
