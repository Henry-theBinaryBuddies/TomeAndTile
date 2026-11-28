// The state the panels read, and the pure function that builds it from the
// server's messages. Nothing here opens a socket or changes game state: it only
// records what the server said. Full tokens, creatures and ability resolutions are
// kept by id, because the initiative bar, the chat log and the character sheet read
// them. Scenes are not kept here (apart from the map target for the map image
// control); the map renderer takes scenes and token positions from the raw frames.
import { Role } from '@tomeandtile/contracts';
import type {
	Creature,
	Encounter,
	Event,
	Member,
	Resolution,
	RollResult,
	Scene,
	ServerMessage,
	Snapshot,
	Token,
	Warning
} from '@tomeandtile/contracts';

/** Most log entries kept in memory. Older ones are dropped. */
export const MAX_LOG_ENTRIES = 1000;

export type LogEntry =
	| { kind: 'chat'; seq: bigint; atMs: number; userId: string; text: string; undone: boolean }
	| { kind: 'roll'; seq: bigint; atMs: number; roll: RollResult; undone: boolean }
	| ResultEntry;

/**
 * One ability use, shown once in the log. The resolution itself is in `PanelState.resolutions`
 * under `resolutionId`, so it always shows the latest state the server sent.
 */
export interface ResultEntry {
	kind: 'result';
	/** Sequence number of the first event for this resolution; the entry's place in the log. */
	seq: bigint;
	/** Sequence numbers of every event that updated it, so an undo of any of them marks it. */
	seqs: bigint[];
	atMs: number;
	resolutionId: string;
	/** The intent that caused it. Empty if the server did not say. */
	clientIntentId: string;
	/** Warnings from the events and the resolution, each once. They never block anything. */
	warnings: Warning[];
	undone: boolean;
}

/** The level of the active scene that a map image is set on. */
export interface MapTarget {
	campaignId: string;
	sceneId: string;
	levelId: string;
	/** The asset id of that level's background now, or '' if it has none. */
	backgroundAssetId: string;
}

/** The first level of a scene. Single-level maps, the only kind in M1, have exactly one. */
export function mapTargetOf(scene: Scene | undefined): MapTarget | null {
	const level = scene?.levels[0];
	if (!scene || !level) return null;
	return {
		campaignId: scene.campaignId,
		sceneId: scene.sceneId,
		levelId: level.levelId,
		backgroundAssetId: level.background?.assetId ?? ''
	};
}

export interface PanelState {
	/** True once a Snapshot has arrived on the current connection. */
	joined: boolean;
	/** Sequence number of the last Snapshot or Event applied. */
	seq: bigint;
	userId: string;
	role: Role;
	members: Member[];
	activeSceneId: string;
	/** Where a map image goes, or null if the active scene is not known or has no level. */
	mapTarget: MapTarget | null;
	/** Chat lines and dice rolls in the order the server sent them. */
	log: LogEntry[];
	encounters: Encounter[];
	/** Every token the server sent, whole, by token id. An empty name means unknown. */
	tokens: Record<string, Token>;
	/** Every creature sheet the server sent to this viewer, by creature id. */
	creatures: Record<string, Creature>;
	/** The latest state of each ability resolution, by resolution id. */
	resolutions: Record<string, Resolution>;
	/**
	 * Warnings the server put on the last token update, with the intent that caused it (for
	 * example `condition_immune` on a SetConditionsIntent). Null if there were none. They never
	 * block anything.
	 */
	tokenWarnings: { clientIntentId: string; warnings: Warning[] } | null;
}

export function emptyState(): PanelState {
	return {
		joined: false,
		seq: 0n,
		userId: '',
		role: Role.UNSPECIFIED,
		members: [],
		activeSceneId: '',
		mapTarget: null,
		log: [],
		encounters: [],
		tokens: {},
		creatures: {},
		resolutions: {},
		tokenWarnings: null
	};
}

/** A Snapshot replaces everything: there is no replay, so earlier chat is gone. */
export function applySnapshot(snapshot: Snapshot): PanelState {
	return {
		joined: true,
		seq: snapshot.seq,
		userId: snapshot.userId,
		role: snapshot.role,
		members: [...snapshot.members],
		activeSceneId: snapshot.activeSceneId,
		mapTarget: mapTargetOf(snapshot.scenes.find((x) => x.sceneId === snapshot.activeSceneId)),
		log: [],
		encounters: [...snapshot.encounters],
		tokens: Object.fromEntries(snapshot.tokens.map((t) => [t.tokenId, t])),
		creatures: Object.fromEntries(snapshot.creatures.map((c) => [c.creatureId, c])),
		resolutions: {},
		tokenWarnings: null
	};
}

/** Append to the log, dropping the oldest entries (and their resolutions) past the limit. */
function appendLog(
	state: PanelState,
	entry: LogEntry
): Pick<PanelState, 'log' | 'resolutions'> {
	const next = [...state.log, entry];
	if (next.length <= MAX_LOG_ENTRIES) return { log: next, resolutions: state.resolutions };
	const dropped = next.slice(0, next.length - MAX_LOG_ENTRIES);
	let resolutions = state.resolutions;
	for (const old of dropped) {
		if (old.kind === 'result' && old.resolutionId in resolutions) {
			const { [old.resolutionId]: _gone, ...rest } = resolutions;
			resolutions = rest;
		}
	}
	return { log: next.slice(dropped.length), resolutions };
}

/** Does this entry stand for the event with this sequence number? */
function entryHasSeq(entry: LogEntry, seq: bigint): boolean {
	return entry.kind === 'result' ? entry.seqs.includes(seq) : entry.seq === seq;
}

function sameWarning(a: Warning, b: Warning): boolean {
	return a.code === b.code && a.message === b.message;
}

function mergeWarnings(existing: Warning[], incoming: readonly Warning[]): Warning[] {
	const merged = [...existing];
	for (const w of incoming) if (!merged.some((m) => sameWarning(m, w))) merged.push(w);
	return merged;
}

/**
 * Apply one Event. Events before the first Snapshot, and events at or below the
 * last applied sequence number, are ignored, so a repeated event changes nothing.
 */
export function applyEvent(state: PanelState, event: Event): PanelState {
	if (!state.joined || event.seq <= state.seq) return state;
	const base = { ...state, seq: event.seq };
	const atMs = Number(event.atMs);
	const kind = event.kind;
	switch (kind.case) {
		case 'chatPosted':
			return {
				...base,
				...appendLog(state, {
					kind: 'chat',
					seq: event.seq,
					atMs,
					userId: kind.value.userId,
					text: kind.value.text,
					undone: false
				})
			};
		case 'diceRolled': {
			const roll = kind.value.roll;
			if (!roll) return base;
			return {
				...base,
				...appendLog(state, { kind: 'roll', seq: event.seq, atMs, roll, undone: false })
			};
		}
		case 'encounterUpdated': {
			const updated = kind.value.encounter;
			if (!updated) return base;
			const known = state.encounters.some((e) => e.encounterId === updated.encounterId);
			return {
				...base,
				encounters: known
					? state.encounters.map((e) => (e.encounterId === updated.encounterId ? updated : e))
					: [...state.encounters, updated]
			};
		}
		case 'encounterEnded':
			return {
				...base,
				encounters: state.encounters.filter((e) => e.encounterId !== kind.value.encounterId)
			};
		case 'tokenUpserted': {
			const token = kind.value.token;
			if (!token) return base;
			return {
				...base,
				tokens: { ...state.tokens, [token.tokenId]: token },
				tokenWarnings:
					event.warnings.length > 0
						? { clientIntentId: event.clientIntentId, warnings: [...event.warnings] }
						: state.tokenWarnings
			};
		}
		case 'tokenMoved': {
			const { tokenId, path, levelId } = kind.value;
			const token = state.tokens[tokenId];
			if (!token) return base;
			const last = path[path.length - 1];
			const moved = {
				...token,
				position: last ?? token.position,
				levelId: levelId !== '' ? levelId : token.levelId
			};
			return { ...base, tokens: { ...state.tokens, [tokenId]: moved } };
		}
		case 'tokenRemoved': {
			const { [kind.value.tokenId]: _removed, ...rest } = state.tokens;
			return { ...base, tokens: rest };
		}
		case 'creatureUpserted': {
			const creature = kind.value.creature;
			if (!creature) return base;
			return { ...base, creatures: { ...state.creatures, [creature.creatureId]: creature } };
		}
		case 'resolutionUpdated': {
			const resolution = kind.value.resolution;
			if (!resolution) return base;
			const id = resolution.resolutionId;
			const warnings = [...event.warnings, ...resolution.warnings];
			const resolutions = { ...state.resolutions, [id]: resolution };
			const at = state.log.findIndex((e) => e.kind === 'result' && e.resolutionId === id);
			if (at === -1) {
				return {
					...base,
					...appendLog(
						{ ...state, resolutions },
						{
							kind: 'result',
							seq: event.seq,
							seqs: [event.seq],
							atMs,
							resolutionId: id,
							clientIntentId: event.clientIntentId,
							warnings: mergeWarnings([], warnings),
							undone: false
						}
					)
				};
			}
			// The same resolution again: the entry stays where it is, the newest state wins.
			const entry = state.log[at] as ResultEntry;
			const updated: ResultEntry = {
				...entry,
				seqs: [...entry.seqs, event.seq],
				clientIntentId: entry.clientIntentId || event.clientIntentId,
				warnings: mergeWarnings(entry.warnings, warnings)
			};
			return {
				...base,
				resolutions,
				log: state.log.map((e, i) => (i === at ? updated : e))
			};
		}
		case 'sceneUpserted': {
			const scene = kind.value.scene;
			if (!scene || scene.sceneId !== state.activeSceneId) return base;
			return { ...base, mapTarget: mapTargetOf(scene) };
		}
		case 'eventUndone':
			return {
				...base,
				log: state.log.map((entry) =>
					entryHasSeq(entry, kind.value.undoneSeq) ? { ...entry, undone: true } : entry
				)
			};
		default:
			// Scene events are not panel state.
			return base;
	}
}

/** Fold one message from the server into the state. */
export function foldMessage(state: PanelState, message: ServerMessage): PanelState {
	const kind = message.kind;
	switch (kind.case) {
		case 'snapshot':
			return applySnapshot(kind.value);
		case 'event':
			return applyEvent(state, kind.value);
		default:
			return state;
	}
}
