// What the sheet, the initiative bar and the GM editor show about conditions. The UI never
// applies, removes or expires a condition (ADR 0005): it shows the server's list, and the editor
// only asks the server to add or remove ids.
import { create } from '@bufbuild/protobuf';
import { Role, SetConditionsIntentSchema, type Creature, type Token } from '@tomeandtile/contracts';
import type { IntentKind } from '../net';
import { SRD_CONDITIONS, conditionName, type ConditionName } from './names';

export const CONDITIONS_UNKNOWN = 'Conditions unknown';

export interface ConditionList {
	/** False only when there is neither a token nor a creature to read the list from. */
	known: boolean;
	items: ConditionName[];
	/** "Prone, Poisoned", "None", or "Conditions unknown". */
	text: string;
}

function listOf(ids: readonly string[]): ConditionList {
	const items = ids.map(conditionName);
	return { known: true, items, text: items.length ? items.map((i) => i.name).join(', ') : 'None' };
}

/**
 * The viewer's token decides. A token sent without conditions has none. With no token the
 * creature's own list is used; with neither, the list is unknown, not empty.
 */
export function conditionList(
	token: Pick<Token, 'conditions'> | undefined,
	creature?: Pick<Creature, 'conditions'>
): ConditionList {
	if (token) return listOf(token.conditions);
	if (creature) return listOf(creature.conditions);
	return { known: false, items: [], text: CONDITIONS_UNKNOWN };
}

/** Condition immunities, listed apart from the active ones. Empty when none are stated. */
export function immunityList(creature: Pick<Creature, 'conditionImmunities'> | undefined): ConditionName[] {
	return (creature?.conditionImmunities ?? []).map(conditionName);
}

export interface ShortList {
	shown: ConditionName[];
	hidden: number;
	/** Every name, for screen readers and for the tooltip and focus text. */
	full: string;
}

/** A row on the initiative bar has little room: show the first few and count the rest. */
export function shortList(items: readonly ConditionName[], max = 2): ShortList {
	return {
		shown: items.slice(0, max),
		hidden: Math.max(items.length - max, 0),
		full: items.map((i) => i.name).join(', ')
	};
}

// ---- announcements ----

/** Sentences for conditions that were added or removed between two states of the same tokens. */
export function conditionChanges(
	before: Readonly<Record<string, Pick<Token, 'name' | 'conditions'>>>,
	after: Readonly<Record<string, Pick<Token, 'name' | 'conditions'>>>
): string[] {
	const out: string[] = [];
	for (const [id, token] of Object.entries(after)) {
		const was = before[id];
		if (!was) continue;
		const who = token.name.trim() || 'Unknown';
		for (const c of token.conditions)
			if (!was.conditions.includes(c)) out.push(`${who} is now ${conditionName(c).name}`);
		for (const c of was.conditions)
			if (!token.conditions.includes(c)) out.push(`${who} is no longer ${conditionName(c).name}`);
	}
	return out;
}

// ---- the GM editor ----

/** Every id the editor offers: the SRD ones, then any other id the token has (so it can be removed). */
export function editorOptions(current: readonly string[]): ConditionName[] {
	const extra = current.filter((c) => !(SRD_CONDITIONS as readonly string[]).includes(c));
	return [...SRD_CONDITIONS, ...extra].map(conditionName);
}

export function toggleCondition(selected: readonly string[], id: string): string[] {
	return selected.includes(id) ? selected.filter((s) => s !== id) : [...selected, id];
}

export interface ConditionDiff {
	add: string[];
	remove: string[];
}

export function conditionDiff(current: readonly string[], selected: readonly string[]): ConditionDiff {
	return {
		add: selected.filter((s) => !current.includes(s)),
		remove: current.filter((c) => !selected.includes(c))
	};
}

/** One intent with the ids to add and to remove, or null when nothing changed (nothing to send). */
export function buildSetConditionsKind(
	tokenId: string,
	current: readonly string[],
	selected: readonly string[]
): IntentKind | null {
	const { add, remove } = conditionDiff(current, selected);
	if (add.length === 0 && remove.length === 0) return null;
	return { case: 'setConditions', value: create(SetConditionsIntentSchema, { tokenId, add, remove }) };
}

/**
 * Shown beside a condition the creature is immune to. It stays selectable: the server decides
 * and answers with a warning (condition_immune) if it was added anyway.
 */
export function immuneNote(id: string, creature: Pick<Creature, 'conditionImmunities'> | undefined): string {
	return creature?.conditionImmunities.includes(id)
		? 'This creature is immune. The server decides.'
		: '';
}

/** The editor is for the GM only. The server also refuses anyone else (not_permitted). */
export function canEditConditions(role: Role): boolean {
	return role === Role.GM;
}

/** Esc closes the editor without sending, except while an input method is composing text. */
export function closesEditor(e: { key: string; isComposing?: boolean }): boolean {
	return e.key === 'Escape' && !e.isComposing;
}
