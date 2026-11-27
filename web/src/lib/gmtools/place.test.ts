import { create } from '@bufbuild/protobuf';
import { describe, expect, it } from 'vitest';
import { Role, CreatureSchema, CreateTokenIntentSchema, RemoveTokenIntentSchema, TokenSchema, Vec2Schema } from '@tomeandtile/contracts';
import type { MapTarget } from '../net';
import {
	buildCreateTokenKind,
	buildRemoveTokenKind,
	canUseGmTools,
	cellCentre,
	creatureEntries,
	parseCell,
	placeBlockedReason,
	removableTokens,
	removeQuestion
} from './place';

const map: MapTarget = {
	campaignId: 'camp',
	sceneId: 's1',
	levelId: 'l1',
	backgroundAssetId: '',
	cellSize: 70,
	gridOffset: { x: 10, y: 20 }
};
const creature = (creatureId: string, name: string, summary = '') =>
	create(CreatureSchema, { creatureId, name, summary });
const creatures = {
	g: creature('g', 'Goblin', 'Small, sneaky'),
	a: creature('a', 'aria', ''),
	o: creature('o', 'Ogre', 'Big'),
	n: creature('n', '  ', '')
};

describe('creatureEntries', () => {
	it('lists every creature by name with its summary', () => {
		const names = creatureEntries(creatures, '').map((c) => c.name);
		expect(names).toEqual(['aria', 'Goblin', 'Ogre', 'Unknown']);
		expect(creatureEntries(creatures, '').find((c) => c.creatureId === 'g')?.summary).toBe('Small, sneaky');
	});

	it('filters by name, in any letter case, and ignores the summary', () => {
		expect(creatureEntries(creatures, 'GOB').map((c) => c.creatureId)).toEqual(['g']);
		expect(creatureEntries(creatures, 'sneaky')).toEqual([]);
		expect(creatureEntries(creatures, '  ')).toHaveLength(4);
	});

	it('keeps markup in a name as plain text', () => {
		const list = creatureEntries({ x: creature('x', '<b>Bold</b>') }, '');
		expect(list[0].name).toBe('<b>Bold</b>');
	});
});

describe('placing a creature', () => {
	it('parses a column or row as a whole number from 0', () => {
		expect(parseCell('0')).toEqual({ ok: true, value: 0 });
		expect(parseCell(' 12 ')).toEqual({ ok: true, value: 12 });
		for (const bad of ['', '-1', '1.5', 'a', '12345']) expect(parseCell(bad).ok, bad).toBe(false);
	});

	it('finds the centre of a cell from the grid size and offset', () => {
		expect(cellCentre(map, 0, 0)).toEqual({ x: 45, y: 55 });
		expect(cellCentre(map, 2, 3)).toEqual({ x: 185, y: 265 });
	});

	it('sends CreateTokenIntent for the active scene and level with the creature and position', () => {
		const kind = buildCreateTokenKind(map, 'g', '2', '3')!;
		expect(kind.case).toBe('createToken');
		expect(kind.value).toEqual(
			create(CreateTokenIntentSchema, {
				sceneId: 's1',
				levelId: 'l1',
				creatureId: 'g',
				position: create(Vec2Schema, { x: 185, y: 265 })
			})
		);
	});

	it('sends nothing, and says why, without a map, a grid or a good cell', () => {
		expect(placeBlockedReason(null, '0', '0')).toMatch(/no active map/);
		expect(placeBlockedReason({ ...map, cellSize: 0 }, '0', '0')).toMatch(/grid size is not known/);
		expect(placeBlockedReason(map, 'x', '0')).toMatch(/^Column/);
		expect(placeBlockedReason(map, '0', '')).toMatch(/^Row/);
		expect(placeBlockedReason(map, '0', '0')).toBe('');
		expect(buildCreateTokenKind(null, 'g', '0', '0')).toBeNull();
		expect(buildCreateTokenKind({ ...map, cellSize: 0 }, 'g', '0', '0')).toBeNull();
		expect(buildCreateTokenKind(map, 'g', 'x', '0')).toBeNull();
	});
});

describe('removing a token', () => {
	const tokens = {
		a: create(TokenSchema, { tokenId: 'a', name: 'Zed', sceneId: 's1' }),
		b: create(TokenSchema, { tokenId: 'b', name: 'Amy', sceneId: 's1' }),
		c: create(TokenSchema, { tokenId: 'c', name: 'Far', sceneId: 's2' }),
		d: create(TokenSchema, { tokenId: 'd', name: '', sceneId: 's1' })
	};

	it('lists the active scene tokens by name', () => {
		expect(removableTokens(tokens, 's1').map((t) => t.name)).toEqual(['Amy', 'Unknown', 'Zed']);
	});

	it('names the token in the question and sends RemoveTokenIntent with its id', () => {
		expect(removeQuestion('Amy')).toBe('Remove Amy from the map?');
		const kind = buildRemoveTokenKind('b');
		expect(kind.case).toBe('removeToken');
		expect(kind.value).toEqual(create(RemoveTokenIntentSchema, { tokenId: 'b' }));
	});
});

describe('who sees the GM tools', () => {
	it('only the GM', () => {
		expect(canUseGmTools(Role.GM)).toBe(true);
		expect(canUseGmTools(Role.PLAYER)).toBe(false);
		expect(canUseGmTools(Role.UNSPECIFIED)).toBe(false);
	});
});
