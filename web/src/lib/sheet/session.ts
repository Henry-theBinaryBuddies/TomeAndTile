// The target picking that is in progress, shared by the Sheet panel and the map. It lives
// in a store, not in the panel, because on a phone the Sheet is closed (and so unmounted)
// while the person taps tokens on the map. The picks and every rule about them stay in
// targeting.ts; this file only holds the current state and connects it to the map.
import { writable, type Readable } from 'svelte/store';
import { picksTargets, pickToken, type TargetingState } from './targeting';

export interface TargetingSession {
	/** The character the ability belongs to. Picking ends if another character is shown. */
	creatureId: string;
	state: TargetingState;
}

const store = writable<TargetingSession | null>(null);

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

interface EscapeEvent {
	key: string;
	preventDefault(): void;
	stopPropagation(): void;
}

/**
 * Escape ends picking. Registered for the whole page (not by the Sheet panel, which may be
 * closed). It stops the event, so on a phone the first Escape ends picking and a second one
 * closes the panel. Returns true if it handled the key.
 */
export function endTargetingOnEscape(e: EscapeEvent): boolean {
	let active = false;
	store.update((s) => {
		active = s !== null;
		return s;
	});
	if (!active || e.key !== 'Escape') return false;
	e.preventDefault();
	e.stopPropagation();
	endTargeting();
	return true;
}
