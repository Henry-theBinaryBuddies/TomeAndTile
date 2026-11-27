// What the GM's buttons on a result card send, and when they are on. Nothing here changes a
// card, a token or a condition: every button sends an intent, and the screen changes when the
// server's event arrives (ADR 0005, ADR 0006). The server checks the role again.
import { create } from '@bufbuild/protobuf';
import {
	Role,
	RollMode,
	SetHitPointsIntentSchema,
	UndoIntentSchema,
	UseAbilityIntentSchema,
	type Token
} from '@tomeandtile/contracts';
import type { IntentKind } from '../net';

/** The GM's buttons are for the GM only, and not on a card that was already undone. */
export function showOverrides(role: Role, undone: boolean): boolean {
	return role === Role.GM && !undone;
}

// ---- undo ----

/**
 * The seq to undo for one ability use: the first event of its group, which is the lowest seq of
 * the card (its ResolutionUpdated). Naming any later seq is refused with `not_first_event`.
 */
export function undoSeq(seqs: readonly bigint[]): bigint | null {
	if (seqs.length === 0) return null;
	return seqs.reduce((lowest, s) => (s < lowest ? s : lowest));
}

export function buildUndoKind(seqs: readonly bigint[]): IntentKind | null {
	const eventSeq = undoSeq(seqs);
	return eventSeq === null
		? null
		: { case: 'undo', value: create(UndoIntentSchema, { eventSeq }) };
}

/** "Undo Longsword by Brannoch on Goblin, Orc?" Names the ability and the targets. */
export function undoQuestion(abilityName: string, sourceName: string, targetNames: readonly string[]): string {
	const on = targetNames.length > 0 ? ` on ${[...new Set(targetNames)].join(', ')}` : '';
	return `Undo ${abilityName} by ${sourceName}${on}?`;
}

// ---- set hit points ----

export type HpInput =
	| { ok: true; current: number; temp: number }
	| { ok: false; reason: string };

function wholeNumber(text: string): number | null {
	const t = text.trim();
	if (!/^\d{1,6}$/.test(t)) return null;
	return Number(t);
}

/** Two whole numbers from 0 up, or nothing is sent. The server checks the limits again. */
export function parseHp(currentText: string, tempText: string): HpInput {
	const current = wholeNumber(currentText);
	if (current === null) return { ok: false, reason: 'Current HP: enter a whole number, 0 or more.' };
	const temp = wholeNumber(tempText);
	if (temp === null) return { ok: false, reason: 'Temporary HP: enter a whole number, 0 or more.' };
	return { ok: true, current, temp };
}

export function buildSetHitPointsKind(tokenId: string, current: number, temp: number): IntentKind {
	return {
		case: 'setHitPoints',
		value: create(SetHitPointsIntentSchema, { tokenId, hpCurrent: current, hpTemp: temp })
	};
}

/** What the HP form starts with: the token's numbers where the GM has them, else blank. */
export function hpPrefill(
	token: Pick<Token, 'hpCurrent' | 'hpTemp'> | undefined
): { current: string; temp: string } {
	return {
		current: token?.hpCurrent !== undefined ? String(token.hpCurrent) : '',
		temp: token?.hpTemp !== undefined ? String(token.hpTemp) : ''
	};
}

// ---- halve ----

export type HalvePrefill =
	| { ok: true; current: number; temp: string; giveBack: number; total: number; note: string }
	| { ok: false; reason: string };

/**
 * A starting figure for the HP form after "Halve": the token's current HP plus the damage that
 * would not have been dealt (total applied damage minus half of it, rounded down), but never
 * above the token's maximum HP.
 *
 * This arithmetic only fills in a form. The GM reads it, confirms or edits it, and the server
 * sets the HP and works out everything that follows from it (ADR 0005). It is never shown as a
 * result and never sent without that confirmation.
 */
export function halvePrefill(
	token: Pick<Token, 'hpCurrent' | 'hpMax' | 'hpTemp'> | undefined,
	appliedTotal: number | null
): HalvePrefill {
	if (token?.hpCurrent === undefined || token.hpMax === undefined || token.hpMax <= 0)
		return { ok: false, reason: 'Halve needs this token’s hit points, and this page does not have them.' };
	if (appliedTotal === null || appliedTotal <= 0)
		return { ok: false, reason: 'There is no damage to halve.' };
	const wanted = token.hpCurrent + (appliedTotal - Math.floor(appliedTotal / 2));
	const current = Math.min(wanted, token.hpMax);
	const giveBack = current - token.hpCurrent;
	return {
		ok: true,
		current,
		temp: token.hpTemp !== undefined ? String(token.hpTemp) : '',
		giveBack,
		total: appliedTotal,
		note: `Gives back ${giveBack} of ${appliedTotal} damage. Check it and change it if needed.`
	};
}

// ---- re-roll ----

/** What was sent to use the ability, remembered by the intent id so a re-roll can send it again. */
export interface UseRecord {
	sourceTokenId: string;
	abilityId: string;
	targetTokenIds: string[];
	mode: RollMode;
}

const sentUses = new Map<string, UseRecord>();
const MAX_REMEMBERED = 200;

export function rememberUse(clientIntentId: string | null, use: UseRecord): void {
	if (!clientIntentId) return;
	sentUses.set(clientIntentId, { ...use, targetTokenIds: [...use.targetTokenIds] });
	// Keep the page light: forget the oldest.
	while (sentUses.size > MAX_REMEMBERED) sentUses.delete(sentUses.keys().next().value as string);
}

export function rememberedUse(clientIntentId: string): UseRecord | null {
	return clientIntentId ? (sentUses.get(clientIntentId) ?? null) : null;
}

export const REROLL_UNKNOWN =
	'Re-roll needs the original targets and roll mode, and this page did not send that use.';

/** Why Re-roll is off, or "" when it can be pressed. */
export function rerollBlockedReason(clientIntentId: string): string {
	return rememberedUse(clientIntentId) ? '' : REROLL_UNKNOWN;
}

export function buildUseAgainKind(use: UseRecord): IntentKind {
	return {
		case: 'useAbility',
		value: create(UseAbilityIntentSchema, {
			sourceTokenId: use.sourceTokenId,
			abilityId: use.abilityId,
			targetTokenIds: use.targetTokenIds,
			mode: use.mode
		})
	};
}

export type RerollNext = 'wait' | 'send_again' | 'stop';

/**
 * Re-roll is undo, then use again. After the undo is sent: if the server refused it, stop and send
 * nothing; once the server says the card is undone, send the use again; until then wait.
 */
export function rerollNext(undoRefused: boolean, cardUndone: boolean): RerollNext {
	if (undoRefused) return 'stop';
	return cardUndone ? 'send_again' : 'wait';
}
