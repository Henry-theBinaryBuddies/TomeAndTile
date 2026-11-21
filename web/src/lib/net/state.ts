// The state the panels read, and the pure function that builds it from the
// server's messages. Nothing here opens a socket or changes game state: it only
// records what the server said. Tokens and scenes are not kept here; the map
// renderer takes those from the raw frames. Two small things are kept: each token's
// name, because the initiative bar needs to say whose turn it is, and the level of the
// active scene that the GM's "Set map image" control puts a background on.
import { Role } from '@tomeandtile/contracts';
import type {
	Encounter,
	Event,
	Member,
	RollResult,
	Scene,
	ServerMessage,
	Snapshot
} from '@tomeandtile/contracts';

/** Most log entries kept in memory. Older ones are dropped. */
export const MAX_LOG_ENTRIES = 1000;

export type LogEntry =
	| { kind: 'chat'; seq: bigint; atMs: number; userId: string; text: string; undone: boolean }
	| { kind: 'roll'; seq: bigint; atMs: number; roll: RollResult; undone: boolean };

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
	/** Token id to name. A token with no name is absent or empty: treat it as unknown. */
	tokenNames: Record<string, string>;
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
		tokenNames: {}
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
		tokenNames: Object.fromEntries(snapshot.tokens.map((t) => [t.tokenId, t.name]))
	};
}

function appendLog(log: LogEntry[], entry: LogEntry): LogEntry[] {
	const next = [...log, entry];
	return next.length > MAX_LOG_ENTRIES ? next.slice(next.length - MAX_LOG_ENTRIES) : next;
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
				log: appendLog(state.log, {
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
				log: appendLog(state.log, { kind: 'roll', seq: event.seq, atMs, roll, undone: false })
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
			return { ...base, tokenNames: { ...state.tokenNames, [token.tokenId]: token.name } };
		}
		case 'tokenRemoved': {
			const { [kind.value.tokenId]: _removed, ...rest } = state.tokenNames;
			return { ...base, tokenNames: rest };
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
					entry.seq === kind.value.undoneSeq ? { ...entry, undone: true } : entry
				)
			};
		default:
			// Token moves, creature and resolution events are not panel state.
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
