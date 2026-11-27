import { create } from '@bufbuild/protobuf';
import { describe, expect, it } from 'vitest';
import { HealthState, TokenSchema } from '@tomeandtile/contracts';
import { healthStateText, healthView } from './view';

const token = (over: Parameters<typeof create<typeof TokenSchema>>[1] = {}) =>
	create(TokenSchema, { tokenId: 't1', name: 'Goblin', ...over });

describe('healthStateText', () => {
	it('puts each state in words', () => {
		expect(healthStateText(HealthState.HEALTHY)).toBe('Healthy');
		expect(healthStateText(HealthState.BLOODIED)).toBe('Bloodied');
		expect(healthStateText(HealthState.NEAR_DEATH)).toBe('Near death');
		expect(healthStateText(HealthState.DOWN)).toBe('Down');
	});

	it('says unknown, never healthy, for an unset or unrecognised state', () => {
		expect(healthStateText(HealthState.UNSPECIFIED)).toBe('Health unknown');
		expect(healthStateText(undefined)).toBe('Health unknown');
		expect(healthStateText(99 as HealthState)).toBe('Health unknown');
	});
});

describe('healthView', () => {
	it('shows the numbers and the state to a viewer the server sent numbers to', () => {
		const v = healthView(
			token({ healthState: HealthState.BLOODIED, hpCurrent: 6, hpMax: 13, hpTemp: 0 })
		);
		expect(v.hp).toBe('6 of 13');
		expect(v.temp).toBeNull();
		expect(v.stateText).toBe('Bloodied');
		expect(v.short).toBe('6 of 13, Bloodied');
		expect(v.label).toBe('6 of 13 hit points, Bloodied');
	});

	it('shows temporary hit points when there are some', () => {
		const v = healthView(
			token({ healthState: HealthState.HEALTHY, hpCurrent: 12, hpMax: 13, hpTemp: 4 })
		);
		expect(v.temp).toBe('+4 temporary');
		expect(v.short).toBe('12 of 13 (+4 temporary), Healthy');
	});

	it('shows only the state to a player looking at a monster', () => {
		const v = healthView(token({ healthState: HealthState.NEAR_DEATH }));
		expect(v.hp).toBeNull();
		expect(v.short).toBe('Near death');
		expect(v.label).toBe('Near death');
	});

	it('shows no numbers, not zero, when the server sent only one of them', () => {
		const v = healthView(token({ healthState: HealthState.HEALTHY, hpCurrent: 5 }));
		expect(v.hp).toBeNull();
		const noMax = healthView(token({ healthState: HealthState.HEALTHY, hpCurrent: 5, hpMax: 0 }));
		expect(noMax.hp).toBeNull();
	});

	it('does not work out a state from the numbers', () => {
		// 1 of 13 would be "near death" by the server's rule, but the server sent no state.
		const v = healthView(token({ hpCurrent: 1, hpMax: 13 }));
		expect(v.stateText).toBe('Health unknown');
		expect(v.short).toBe('1 of 13, Health unknown');
	});

	it('marks Down so it does not rely on colour', () => {
		const v = healthView(token({ healthState: HealthState.DOWN, hpCurrent: 0, hpMax: 13 }));
		expect(v.down).toBe(true);
		expect(v.short).toBe('0 of 13, Down');
		expect(healthView(token({ healthState: HealthState.HEALTHY })).down).toBe(false);
	});

	it('says health is unknown when there is no token', () => {
		const v = healthView(undefined);
		expect(v.short).toBe('Health unknown');
		expect(v.down).toBe(false);
		expect(v.hp).toBeNull();
	});
});
