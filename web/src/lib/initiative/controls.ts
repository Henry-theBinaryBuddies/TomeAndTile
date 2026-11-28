// What the encounter controls send, and who sees them. The UI never rolls initiative, sorts the
// order or advances a turn (ADR 0005): it asks, and the bar changes only when the server's
// EncounterUpdated or EncounterEnded arrives. The server has the last word on every permission.
import { create } from '@bufbuild/protobuf';
import {
	EndEncounterIntentSchema,
	EndTurnIntentSchema,
	Role,
	SetInitiativeIntentSchema,
	StartEncounterIntentSchema,
	type Combatant,
	type Token
} from '@tomeandtile/contracts';
import type { IntentKind } from '../net';

// ---- start ----

export interface StartChoice {
	tokenId: string;
	name: string;
	/** True for a token with a character or monster behind it; false for props. */
	isCreature: boolean;
}

/** The tokens of the active scene, by name, for the "who is in the fight" checklist. */
export function startChoices(
	tokens: Readonly<Record<string, Pick<Token, 'tokenId' | 'name' | 'sceneId' | 'creatureId'>>>,
	activeSceneId: string
): StartChoice[] {
	return Object.values(tokens)
		.filter((t) => activeSceneId === '' || t.sceneId === activeSceneId)
		.map((t) => ({
			tokenId: t.tokenId,
			name: t.name.trim() || 'Unknown',
			isCreature: t.creatureId !== ''
		}))
		.sort((a, b) => a.name.localeCompare(b.name) || a.tokenId.localeCompare(b.tokenId));
}

/** Ids to tick for "Select all creatures". */
export function creatureIds(choices: readonly StartChoice[]): string[] {
	return choices.filter((c) => c.isCreature).map((c) => c.tokenId);
}

export const NOTHING_TICKED = 'Tick at least one token to start.';

/** Why Start is off, or "" when it can be pressed. */
export function startBlockedReason(ticked: readonly string[], activeSceneId: string): string {
	if (activeSceneId === '') return 'There is no active map to fight on.';
	return ticked.length === 0 ? NOTHING_TICKED : '';
}

/** The ticked ids go in checklist order, not tick order, so the same ticks send the same intent. */
export function buildStartKind(
	activeSceneId: string,
	choices: readonly StartChoice[],
	ticked: readonly string[]
): IntentKind | null {
	if (startBlockedReason(ticked, activeSceneId) !== '') return null;
	const tokenIds = choices.map((c) => c.tokenId).filter((id) => ticked.includes(id));
	if (tokenIds.length === 0) return null;
	return {
		case: 'startEncounter',
		value: create(StartEncounterIntentSchema, { sceneId: activeSceneId, tokenIds })
	};
}

// ---- end turn, end encounter ----

/** The GM, or someone who controls the current combatant's token. The server decides in the end. */
export function canEndTurn(
	role: Role,
	userId: string,
	currentToken: Pick<Token, 'controllerUserIds'> | undefined
): boolean {
	if (role === Role.GM) return true;
	return userId !== '' && !!currentToken && currentToken.controllerUserIds.includes(userId);
}

export function canControlEncounter(role: Role): boolean {
	return role === Role.GM;
}

export function buildEndTurnKind(encounterId: string): IntentKind {
	return { case: 'endTurn', value: create(EndTurnIntentSchema, { encounterId }) };
}

export function buildEndEncounterKind(encounterId: string): IntentKind {
	return { case: 'endEncounter', value: create(EndEncounterIntentSchema, { encounterId }) };
}

// ---- set initiative ----

export type InitiativeInput = { ok: true; value: number } | { ok: false; reason: string };

const INT32_MAX = 2_147_483_647;

/** A whole number or nothing: empty, non-numeric and out-of-range input is never sent. */
export function parseInitiative(text: string): InitiativeInput {
	const t = text.trim();
	if (t === '') return { ok: false, reason: 'Enter a whole number.' };
	if (!/^[+-]?\d+$/.test(t)) return { ok: false, reason: 'Enter a whole number.' };
	const value = Number(t);
	if (!Number.isSafeInteger(value) || Math.abs(value) > INT32_MAX)
		return { ok: false, reason: 'That number is too large.' };
	return { ok: true, value };
}

export function buildSetInitiativeKind(
	encounterId: string,
	tokenId: string,
	initiative: number,
	order: number
): IntentKind {
	return {
		case: 'setInitiative',
		value: create(SetInitiativeIntentSchema, { encounterId, tokenId, initiative, order })
	};
}

export type MoveDirection = 'up' | 'down';

export interface InitiativeChange {
	tokenId: string;
	initiative: number;
	order: number;
}

/** The neighbour a combatant can swap with: the next one up or down that has the same initiative. */
function tiedNeighbour(combatants: readonly Combatant[], index: number, dir: MoveDirection): number | null {
	const at = combatants[index];
	const other = index + (dir === 'up' ? -1 : 1);
	if (!at || other < 0 || other >= combatants.length) return null;
	if (combatants[other].initiative !== at.initiative) return null;
	// A token that appears twice cannot be told apart by `token_id`, so it is not moved.
	const twice = (id: string) => combatants.filter((c) => c.tokenId === id).length > 1;
	return twice(at.tokenId) || twice(combatants[other].tokenId) ? null : other;
}

export function canMove(combatants: readonly Combatant[], index: number, dir: MoveDirection): boolean {
	return tiedNeighbour(combatants, index, dir) !== null;
}

/**
 * Moving a combatant past one with the same initiative. The server sorts by initiative and then
 * by `order` (lower first), so this only asks for new tie-break numbers: the tied group is laid
 * out in the new sequence and numbered 0, 1, 2 ..., and one SetInitiativeIntent is sent for each
 * combatant whose number changes. The bar changes when the server answers.
 */
export function moveChanges(
	combatants: readonly Combatant[],
	index: number,
	dir: MoveDirection
): InitiativeChange[] {
	const other = tiedNeighbour(combatants, index, dir);
	if (other === null) return [];
	const initiative = combatants[index].initiative;
	let start = index;
	while (start > 0 && combatants[start - 1].initiative === initiative) start--;
	let end = index;
	while (end + 1 < combatants.length && combatants[end + 1].initiative === initiative) end++;
	const group = combatants.slice(start, end + 1);
	const a = index - start;
	const b = other - start;
	[group[a], group[b]] = [group[b], group[a]];
	const changes: InitiativeChange[] = [];
	group.forEach((c, i) => {
		if (c.order !== i) changes.push({ tokenId: c.tokenId, initiative, order: i });
	});
	return changes;
}
