// The target picking that is in progress, shared by the Sheet panel and the map. It lives
// in a store, not in the panel, because on a phone the Sheet is closed (and so unmounted)
// while the person taps tokens on the map. The picks and every rule about them stay in
// targeting.ts; this file only holds the current state and connects it to the map.
//
// The Sheet's own choices (which character, which token acts) are kept here too, for the same
// reason: the Sheet's body is unmounted when the panel is collapsed or, on a phone, closed, and
// picking must not end or change character when it comes back.
import { get, writable, type Readable } from 'svelte/store';
import { picksTargets, pickToken, type TargetingState } from './targeting';
import { sheetChoices, type SheetChoice } from './view';

export interface TargetingSession {
	/** The character the ability belongs to. Picking ends if another character is shown. */
	creatureId: string;
	state: TargetingState;
}

const store = writable<TargetingSession | null>(null);

/** Whose sheet is shown, and which of that character's tokens acts. */
export interface SheetSelection {
	creatureId: string;
	/** The token chosen for `creatureId` when it has more than one; '' if none was chosen. */
	tokenId: string;
}

const selection = writable<SheetSelection>({ creatureId: '', tokenId: '' });

export const sheetSelection: Readable<SheetSelection> = { subscribe: selection.subscribe };

/** Show another character's sheet. The token choice belongs to a character, so it is cleared. */
export function selectCreature(creatureId: string): void {
	selection.update((s) => (s.creatureId === creatureId ? s : { creatureId, tokenId: '' }));
}

/** Choose which of a character's tokens acts. */
export function selectActingToken(creatureId: string, tokenId: string): void {
	selection.set({ creatureId, tokenId });
}

/** The character to show: the selected one, or the first when none is selected or it is gone. */
export function selectedChoice(choices: readonly SheetChoice[], sel: SheetSelection): SheetChoice | undefined {
	return choices.find((c) => c.creatureId === sel.creatureId) ?? choices[0];
}

/** The token choice for the character on screen; a choice made for another character is ignored. */
export function chosenTokenFor(sel: SheetSelection, creatureId: string): string {
	return sel.creatureId === creatureId ? sel.tokenId : '';
}

/** The picking in progress, or null. */
export const targetingSession: Readable<TargetingSession | null> = { subscribe: store.subscribe };

export function beginTargeting(creatureId: string, state: TargetingState): void {
	store.set({ creatureId, state });
}

/** Replace the picks (the list changed them). Does nothing when nothing is in progress. */
export function updateTargeting(change: (state: TargetingState) => TargetingState): void {
	store.update((s) => (s ? { ...s, state: change(s.state) } : s));
}

/** A token was clicked on the map: the same rule as clicking its name in the list. */
export function pickOnMap(tokenId: string): void {
	updateTargeting((state) => pickToken(state, tokenId));
}

/** Confirm, Cancel, Escape, or a different character: picking is over. */
export function endTargeting(): void {
	store.set(null);
}

/** Token ids to ring on the map, each once, in pick order. */
export function highlightIds(state: TargetingState): string[] {
	return [...new Set(state.picks)];
}

/** What the map should be doing for this session. */
export function mapPickingFor(session: TargetingSession | null): { enabled: boolean; highlights: string[] } {
	if (!session || !picksTargets(session.state)) return { enabled: false, highlights: [] };
	return { enabled: true, highlights: highlightIds(session.state) };
}

/**
 * Picking ends if its character is no longer one the viewer can use (a Snapshot removed it, or
 * the connection restarted). The Sheet checks this too, but only while it is mounted; this one
 * also covers a closed Sheet, which would otherwise leave the map in picking mode with stale
 * rings until Escape. Returns the function that stops watching.
 */
export function endTargetingWhenGone(game: Readable<Parameters<typeof sheetChoices>[0]>): () => void {
	return game.subscribe((gameState) => {
		const session = get(store);
		if (!session) return;
		if (!sheetChoices(gameState).some((c) => c.creatureId === session.creatureId)) endTargeting();
	});
}

/** Keeps a map in step with the session: picking on or off, and which tokens are ringed. */
export function followTargeting(
	map: { setPicking(enabled: boolean): void; setHighlights(tokenIds: string[]): void },
	source: Readable<TargetingSession | null> = targetingSession
): () => void {
	return source.subscribe((session) => {
		const { enabled, highlights } = mapPickingFor(session);
		map.setPicking(enabled);
		map.setHighlights(highlights);
	});
}

interface EscapeEvent {
	key: string;
	/** True while an input method (IME) is composing text; Escape then belongs to the IME. */
	isComposing?: boolean;
	preventDefault(): void;
	stopPropagation(): void;
}

/**
 * Escape ends picking. Registered for the whole page (not by the Sheet panel, which may be
 * closed). It stops the event, so on a phone the first Escape ends picking and a second one
 * closes the panel. Returns true if it handled the key.
 */
export function endTargetingOnEscape(e: EscapeEvent): boolean {
	if (e.key !== 'Escape' || e.isComposing || get(store) === null) return false;
	e.preventDefault();
	e.stopPropagation();
	endTargeting();
	return true;
}
