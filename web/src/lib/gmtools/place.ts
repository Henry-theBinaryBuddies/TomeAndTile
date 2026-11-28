// Putting a creature on the map and taking a token off it. The server mints the token id and
// answers with TokenUpserted or TokenRemoved: the map list changes only then (ADR 0005).
import { create } from '@bufbuild/protobuf';
import {
	CreateTokenIntentSchema,
	Role,
	RemoveTokenIntentSchema,
	Vec2Schema,
	type Creature,
	type Token
} from '@tomeandtile/contracts';
import type { IntentKind, MapTarget } from '../net';

export interface CreatureEntry {
	creatureId: string;
	name: string;
	summary: string;
}

/** Every creature the viewer has, by name, narrowed by what the GM typed. Text only. */
export function creatureEntries(
	creatures: Readonly<Record<string, Pick<Creature, 'creatureId' | 'name' | 'summary'>>>,
	filter: string
): CreatureEntry[] {
	const needle = filter.trim().toLowerCase();
	return Object.values(creatures)
		.map((c) => ({
			creatureId: c.creatureId,
			name: c.name.trim() || 'Unknown',
			summary: c.summary.trim()
		}))
		.filter((c) => needle === '' || c.name.toLowerCase().includes(needle))
		.sort((a, b) => a.name.localeCompare(b.name) || a.creatureId.localeCompare(b.creatureId));
}

export type CellInput = { ok: true; value: number } | { ok: false; reason: string };

/** A column or row number: a whole number from 0 up, nothing else. */
export function parseCell(text: string): CellInput {
	const t = text.trim();
	if (!/^\d{1,4}$/.test(t)) return { ok: false, reason: 'Enter a whole number, 0 or more.' };
	return { ok: true, value: Number(t) };
}

export const GRID_UNKNOWN = 'The map grid size is not known yet, so a cell cannot be chosen.';

/** Why Place is off, or "" when it can be pressed. */
export function placeBlockedReason(
	map: Pick<MapTarget, 'sceneId' | 'cellSize'> | null,
	column: string,
	row: string
): string {
	if (!map) return 'There is no active map to place on.';
	if (!(map.cellSize > 0)) return GRID_UNKNOWN;
	const c = parseCell(column);
	if (!c.ok) return `Column: ${c.reason}`;
	const r = parseCell(row);
	if (!r.ok) return `Row: ${r.reason}`;
	return '';
}

/**
 * The centre of a grid cell in scene units. This only turns the cell the GM typed into the
 * point the server expects; the server does not snap it.
 */
export function cellCentre(
	map: Pick<MapTarget, 'cellSize' | 'gridOffset'>,
	column: number,
	row: number
): { x: number; y: number } {
	return {
		x: map.gridOffset.x + (column + 0.5) * map.cellSize,
		y: map.gridOffset.y + (row + 0.5) * map.cellSize
	};
}

export function buildCreateTokenKind(
	map: MapTarget | null,
	creatureId: string,
	column: string,
	row: string
): IntentKind | null {
	if (!map || placeBlockedReason(map, column, row) !== '') return null;
	const c = parseCell(column);
	const r = parseCell(row);
	if (!c.ok || !r.ok) return null;
	const at = cellCentre(map, c.value, r.value);
	return {
		case: 'createToken',
		value: create(CreateTokenIntentSchema, {
			sceneId: map.sceneId,
			levelId: map.levelId,
			creatureId,
			position: create(Vec2Schema, at)
		})
	};
}

export interface RemovableToken {
	tokenId: string;
	name: string;
}

/** The tokens of the active scene, by name. */
export function removableTokens(
	tokens: Readonly<Record<string, Pick<Token, 'tokenId' | 'name' | 'sceneId'>>>,
	activeSceneId: string
): RemovableToken[] {
	return Object.values(tokens)
		.filter((t) => activeSceneId === '' || t.sceneId === activeSceneId)
		.map((t) => ({ tokenId: t.tokenId, name: t.name.trim() || 'Unknown' }))
		.sort((a, b) => a.name.localeCompare(b.name) || a.tokenId.localeCompare(b.tokenId));
}

export function removeQuestion(name: string): string {
	return `Remove ${name} from the map?`;
}

export function buildRemoveTokenKind(tokenId: string): IntentKind {
	return { case: 'removeToken', value: create(RemoveTokenIntentSchema, { tokenId }) };
}

/** The GM tools are for the GM only. The server also refuses anyone else (not_permitted). */
export function canUseGmTools(role: Role): boolean {
	return role === Role.GM;
}
