import { create } from '@bufbuild/protobuf';
import { describe, expect, it } from 'vitest';
import { CombatantSchema, EncounterSchema, HealthState } from '@tomeandtile/contracts';
import { currentEncounter, initiativeView, nameFor, turnAnnouncement } from './view';

const named = (m: Record<string, string>) =>
	Object.fromEntries(Object.entries(m).map(([id, name]) => [id, { name }]));
const names = named({ a: 'Aria', g: 'Goblin', gb: 'Goblin Boss' });

const encounter = (over: Parameters<typeof create<typeof EncounterSchema>>[1] = {}) =>
	create(EncounterSchema, {
		encounterId: 'enc-1',
		sceneId: 'scene-1',
		round: 2,
		turnIndex: 1,
		combatants: [
			create(CombatantSchema, { tokenId: 'gb', initiative: 18 }),
			create(CombatantSchema, { tokenId: 'a', initiative: 15 }),
			create(CombatantSchema, { tokenId: 'g', initiative: 9 })
		],
		...over
	});

describe('initiativeView', () => {
	it('keeps the server order and marks the current turn from turn_index', () => {
		const v = initiativeView(encounter(), names);
		expect(v.rows.map((r) => r.name)).toEqual(['Goblin Boss', 'Aria', 'Goblin']);
		expect(v.rows.map((r) => r.current)).toEqual([false, true, false]);
		expect(v.currentName).toBe('Aria');
		expect(v.round).toBe(2);
	});

	it('shows the server initiative numbers without re-sorting', () => {
		// Deliberately out of numeric order: the server's order wins.
		const e = encounter({
			combatants: [
				create(CombatantSchema, { tokenId: 'a', initiative: 5 }),
				create(CombatantSchema, { tokenId: 'g', initiative: 20 })
			],
			turnIndex: 0
		});
		expect(initiativeView(e, names).rows.map((r) => r.initiative)).toEqual([5, 20]);
	});

	it('says Unknown, never a guess, for a combatant whose name is missing', () => {
		const v = initiativeView(encounter(), named({ a: 'Aria' }));
		expect(v.rows.map((r) => r.name)).toEqual(['Unknown', 'Aria', 'Unknown']);
	});

	it('lists a creature that acts twice twice, with distinct positions', () => {
		const e = encounter({
			combatants: [
				create(CombatantSchema, { tokenId: 'a', initiative: 20 }),
				create(CombatantSchema, { tokenId: 'g', initiative: 12 }),
				create(CombatantSchema, { tokenId: 'a', initiative: 3 })
			],
			turnIndex: 2
		});
		const v = initiativeView(e, names);
		expect(v.rows.map((r) => r.index)).toEqual([0, 1, 2]);
		expect(v.rows.map((r) => r.current)).toEqual([false, false, true]);
	});

	it('has no current turn when turn_index points past the end', () => {
		const v = initiativeView(encounter({ turnIndex: 7 }), names);
		expect(v.rows.some((r) => r.current)).toBe(false);
		expect(v.currentName).toBeNull();
	});

	it('handles an encounter with no combatants', () => {
		const v = initiativeView(encounter({ combatants: [], turnIndex: 0 }), names);
		expect(v.rows).toEqual([]);
		expect(v.currentName).toBeNull();
	});

	it('treats names from the server as plain text', () => {
		const v = initiativeView(encounter(), named({ gb: '<img src=x>', a: 'Aria' }));
		expect(v.rows[0].name).toBe('<img src=x>');
	});
});

describe('currentEncounter', () => {
	const e1 = encounter({ encounterId: 'e1', sceneId: 's1' });
	const e2 = encounter({ encounterId: 'e2', sceneId: 's2' });

	it('prefers the encounter in the active scene', () => {
		expect(currentEncounter([e1, e2], 's2')?.encounterId).toBe('e2');
	});
	it('falls back to the first encounter', () => {
		expect(currentEncounter([e1, e2], 'other')?.encounterId).toBe('e1');
		expect(currentEncounter([e1, e2], '')?.encounterId).toBe('e1');
	});
	it('is null when there is none', () => {
		expect(currentEncounter([], 's1')).toBeNull();
	});
});

describe('nameFor and turnAnnouncement', () => {
	it('trims and falls back to Unknown', () => {
		expect(nameFor(named({ a: '  Aria ' }), 'a')).toBe('Aria');
		expect(nameFor(named({ a: ' ' }), 'a')).toBe('Unknown');
		expect(nameFor({}, 'a')).toBe('Unknown');
	});

	it('announces the round and whose turn it is', () => {
		expect(turnAnnouncement(initiativeView(encounter(), names))).toBe("Round 2. It is Aria's turn.");
		expect(turnAnnouncement(initiativeView(encounter({ turnIndex: 9 }), names))).toBe('Round 2');
	});
});

describe('initiative rows show health', () => {
	const tokens = {
		a: { name: 'Aria', healthState: HealthState.BLOODIED, hpCurrent: 6, hpMax: 13, hpTemp: 0 },
		gb: { name: 'Goblin Boss', healthState: HealthState.NEAR_DEATH },
		g: { name: 'Goblin', healthState: HealthState.DOWN }
	};

	it('shows numbers where the server sent them and the state where it did not', () => {
		const rows = initiativeView(encounter(), tokens).rows;
		expect(rows.map((r) => r.health.short)).toEqual(['Near death', '6 of 13, Bloodied', 'Down']);
	});

	it('marks Down so it is not colour alone', () => {
		const rows = initiativeView(encounter(), tokens).rows;
		expect(rows.map((r) => r.health.down)).toEqual([false, false, true]);
	});

	it('says health is unknown when the token has no state', () => {
		const rows = initiativeView(encounter(), names).rows;
		expect(rows[0].health.short).toBe('Health unknown');
	});
});
