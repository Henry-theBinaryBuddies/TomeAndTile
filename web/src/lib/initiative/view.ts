// What the initiative bar shows, built from the server's `Encounter`. The UI
// never orders, rolls or advances anything: it shows the server's order, the
// server's `turn_index` and the server's round.
import type { Encounter } from '@tomeandtile/contracts';

export interface InitiativeRow {
	/** Position in the server's order. A creature that acts twice appears twice. */
	index: number;
	tokenId: string;
	/** The token's name, or "Unknown" if the server did not send one. */
	name: string;
	/** The server's initiative number. */
	initiative: number;
	current: boolean;
}

export interface InitiativeView {
	encounterId: string;
	round: number;
	rows: InitiativeRow[];
	/** Whose turn it is, or null if `turn_index` points at nobody. */
	currentName: string | null;
}

/** The encounter to show: the one in the active scene, else the first, else none. */
export function currentEncounter(
	encounters: readonly Encounter[],
	activeSceneId: string
): Encounter | null {
	return (
		encounters.find((e) => activeSceneId !== '' && e.sceneId === activeSceneId) ??
		encounters[0] ??
		null
	);
}

export function nameFor(tokenNames: Readonly<Record<string, string>>, tokenId: string): string {
	return tokenNames[tokenId]?.trim() || 'Unknown';
}

export function initiativeView(
	encounter: Encounter,
	tokenNames: Readonly<Record<string, string>>
): InitiativeView {
	const rows = encounter.combatants.map((c, index) => ({
		index,
		tokenId: c.tokenId,
		name: nameFor(tokenNames, c.tokenId),
		initiative: c.initiative,
		current: index === encounter.turnIndex
	}));
	return {
		encounterId: encounter.encounterId,
		round: encounter.round,
		rows,
		currentName: rows.find((r) => r.current)?.name ?? null
	};
}

/** One sentence for screen readers, said when the turn or the round changes. */
export function turnAnnouncement(view: InitiativeView): string {
	return view.currentName === null
		? `Round ${view.round}`
		: `Round ${view.round}. It is ${view.currentName}'s turn.`;
}
