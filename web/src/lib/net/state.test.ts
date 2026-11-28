import { describe, expect, it } from 'vitest';
import { create } from '@bufbuild/protobuf';
import {
	CreatureSchema,
	EventSchema,
	HitOutcome,
	ResolutionSchema,
	ResolutionState,
	Role,
	ServerMessageSchema,
	TargetOutcomeSchema,
	TokenSchema,
	Vec2Schema,
	WarningSchema
} from '@tomeandtile/contracts';
import { applyEvent, emptyState, foldMessage, MAX_LOG_ENTRIES } from './state';
import {
	abilityEventMessage,
	chatKind,
	creatureUpsertedKind,
	resolutionKind,
	tokenMovedKind,
	encounterEndedKind,
	encounterUpdatedKind,
	eventMessage,
	rollKind,
	sceneOf,
	sceneUpsertedKind,
	snapshotMessage,
	tokenRemovedKind,
	tokenUpsertedKind,
	undoneKind
} from './testing';

const joined = () => foldMessage(emptyState(), snapshotMessage(5n));
const fold = (state: ReturnType<typeof joined>, seq: bigint, kind: Parameters<typeof eventMessage>[1]) =>
	foldMessage(state, eventMessage(seq, kind));

describe('snapshot', () => {
	it('sets identity, role, members, scene and encounters', () => {
		const s = joined();
		expect(s.joined).toBe(true);
		expect(s.seq).toBe(5n);
		expect(s.userId).toBe('mock-player');
		expect(s.role).toBe(Role.PLAYER);
		expect(s.members.map((m) => m.displayName)).toEqual(['Gale', 'Pip']);
		expect(s.activeSceneId).toBe('scene-1');
		expect(s.encounters.map((e) => e.encounterId)).toEqual(['enc-1']);
		expect(s.log).toEqual([]);
	});

	it('replaces earlier state, including the log', () => {
		let s = fold(joined(), 6n, chatKind('mock-gm', 'hello'));
		expect(s.log).toHaveLength(1);
		s = foldMessage(s, snapshotMessage(20n));
		expect(s.seq).toBe(20n);
		expect(s.log).toEqual([]);
	});

	it('starts not joined', () => {
		expect(emptyState().joined).toBe(false);
	});
});

describe('events', () => {
	it('are ignored before the first snapshot', () => {
		const s = fold(emptyState(), 1n, chatKind('u', 'early'));
		expect(s).toEqual(emptyState());
	});

	it('append chat lines and rolls in order', () => {
		let s = fold(joined(), 6n, chatKind('mock-gm', 'hello'));
		s = fold(s, 7n, rollKind('r1', 17));
		expect(s.seq).toBe(7n);
		expect(s.log.map((e) => e.kind)).toEqual(['chat', 'roll']);
		const [chat, roll] = s.log;
		expect(chat.kind === 'chat' && chat.text).toBe('hello');
		expect(chat.kind === 'chat' && chat.userId).toBe('mock-gm');
		expect(roll.kind === 'roll' && roll.roll.total).toBe(17);
	});

	it('at or below the last applied sequence change nothing', () => {
		const s = fold(joined(), 6n, chatKind('u', 'once'));
		expect(fold(s, 6n, chatKind('u', 'again'))).toBe(s);
		expect(fold(s, 3n, chatKind('u', 'old'))).toBe(s);
	});

	it('does not change the input state', () => {
		const before = joined();
		const snapshot = structuredClone({ ...before, log: [...before.log] });
		fold(before, 6n, chatKind('u', 'x'));
		expect(before.log).toEqual(snapshot.log);
		expect(before.seq).toBe(5n);
	});

	it('track encounters: add, update and end', () => {
		let s = fold(joined(), 6n, encounterUpdatedKind('enc-1', 2));
		expect(s.encounters).toHaveLength(1);
		expect(s.encounters[0].round).toBe(2);
		s = fold(s, 7n, encounterUpdatedKind('enc-2', 1));
		expect(s.encounters.map((e) => e.encounterId)).toEqual(['enc-1', 'enc-2']);
		s = fold(s, 8n, encounterEndedKind('enc-1'));
		expect(s.encounters.map((e) => e.encounterId)).toEqual(['enc-2']);
	});

	it('mark an undone entry', () => {
		let s = fold(joined(), 6n, chatKind('u', 'oops'));
		s = fold(s, 7n, undoneKind(6n));
		expect(s.log[0].undone).toBe(true);
	});

	it('that are not panel state only advance the sequence', () => {
		const s = fold(joined(), 6n, sceneUpsertedKind());
		expect(s.seq).toBe(6n);
		expect(s.log).toEqual([]);
	});

	it('keep at most MAX_LOG_ENTRIES, dropping the oldest', () => {
		let s = joined();
		for (let i = 1; i <= MAX_LOG_ENTRIES + 5; i++) {
			s = applyEvent(s, eventMessage(5n + BigInt(i), chatKind('u', `m${i}`)).kind.value as never);
		}
		expect(s.log).toHaveLength(MAX_LOG_ENTRIES);
		const first = s.log[0];
		expect(first.kind === 'chat' && first.text).toBe('m6');
	});
});

describe('map target (where a map image goes)', () => {
	const withScene = (...scenes: ReturnType<typeof sceneOf>[]) =>
		foldMessage(emptyState(), snapshotMessage(5n, [], scenes));

	it('is null before a Snapshot and when the active scene is unknown', () => {
		expect(emptyState().mapTarget).toBeNull();
		expect(withScene().mapTarget).toBeNull();
		expect(withScene(sceneOf('other-scene', 'camp-1')).mapTarget).toBeNull();
	});

	it('is the first level of the active scene, with its campaign and current background', () => {
		const s = withScene(
			sceneOf('other-scene', 'camp-2', ['x']),
			sceneOf('scene-1', 'camp-1', ['level-a', 'level-b'], 'asset-1')
		);
		expect(s.mapTarget).toEqual({
			campaignId: 'camp-1',
			sceneId: 'scene-1',
			levelId: 'level-a',
			backgroundAssetId: 'asset-1'
		});
	});

	it('has an empty background id when the level has none, and is null for a scene with no level', () => {
		expect(withScene(sceneOf('scene-1', 'camp-1')).mapTarget?.backgroundAssetId).toBe('');
		expect(withScene(sceneOf('scene-1', 'camp-1', [])).mapTarget).toBeNull();
	});

	it('follows a scene update for the active scene, and ignores other scenes', () => {
		let s = withScene(sceneOf('scene-1', 'camp-1'));
		s = fold(s, 6n, sceneUpsertedKind(sceneOf('scene-1', 'camp-1', ['level-1'], 'asset-9')));
		expect(s.mapTarget?.backgroundAssetId).toBe('asset-9');
		s = fold(s, 7n, sceneUpsertedKind(sceneOf('elsewhere', 'camp-1', ['l'], 'asset-0')));
		expect(s.mapTarget?.backgroundAssetId).toBe('asset-9');
		expect(s.seq).toBe(7n);
	});

	it('is replaced by a new Snapshot', () => {
		let s = withScene(sceneOf('scene-1', 'camp-1', ['level-1'], 'asset-9'));
		s = foldMessage(s, snapshotMessage(20n, [], []));
		expect(s.mapTarget).toBeNull();
	});
});

const fullToken = (over: NonNullable<Parameters<typeof create<typeof TokenSchema>>[1]> = {}) =>
	create(TokenSchema, {
		tokenId: 't1',
		sceneId: 'scene-1',
		levelId: 'ground',
		creatureId: 'c1',
		name: 'Brannoch',
		position: create(Vec2Schema, { x: 2, y: 3 }),
		hpCurrent: 11,
		hpMax: 12,
		hpTemp: 0,
		conditions: ['prone'],
		controllerUserIds: ['mock-player'],
		...over
	});

const withTokens = () => foldMessage(emptyState(), snapshotMessage(5n, [fullToken(), ['t-gob', 'Goblin']]));

describe('tokens', () => {
	it('keeps every field the Snapshot sends, by token id', () => {
		const t = withTokens().tokens['t1'];
		expect(t.creatureId).toBe('c1');
		expect(t.controllerUserIds).toEqual(['mock-player']);
		expect(t.position).toMatchObject({ x: 2, y: 3 });
		expect(t.levelId).toBe('ground');
		expect(t.hpCurrent).toBe(11);
		expect(t.hpMax).toBe(12);
		expect(t.conditions).toEqual(['prone']);
		expect(Object.keys(withTokens().tokens).sort()).toEqual(['t-gob', 't1']);
	});

	it('leaves an HP the server did not send unset, not zero', () => {
		const s = foldMessage(emptyState(), snapshotMessage(5n, [['t-gob', 'Goblin']]));
		expect(s.tokens['t-gob'].hpCurrent).toBeUndefined();
	});

	it('are replaced whole by an upsert, not merged into the old token, and added when new', () => {
		let s = withTokens();
		// Before: the token has HP, a position, a level, a creature, conditions and a controller.
		expect(s.tokens['t1'].hpCurrent).toBe(11);
		// The event comes from the generated types and sets only the id and the name, so every
		// other field is unset. A reducer that merged it into the old token would keep them all.
		s = fold(s, 6n, tokenUpsertedKind('t1', 'Brannoch the Bold'));
		const t = s.tokens['t1'];
		expect(t.name).toBe('Brannoch the Bold');
		expect(t.hpCurrent).toBeUndefined();
		expect(t.hpMax).toBeUndefined();
		expect(t.position).toBeUndefined();
		expect(t.conditions).toEqual([]);
		expect(t.controllerUserIds).toEqual([]);
		expect(t.levelId).toBe('');
		expect(t.sceneId).toBe('');
		expect(t.creatureId).toBe('');
		// Other tokens are untouched, and a new id is added.
		expect(s.tokens['t-gob'].name).toBe('Goblin');
		s = fold(s, 7n, tokenUpsertedKind('t-new', 'Orc'));
		expect(s.tokens['t-new'].name).toBe('Orc');
	});

	it('move to the last point of the path, and to the new level when one is given', () => {
		let s = withTokens();
		s = fold(s, 6n, tokenMovedKind('t1', [[3, 3], [4, 3], [9, 8]]));
		expect(s.tokens['t1'].position).toMatchObject({ x: 9, y: 8 });
		expect(s.tokens['t1'].levelId).toBe('ground');
		s = fold(s, 7n, tokenMovedKind('t1', [[1, 1]], 'upper'));
		expect(s.tokens['t1'].position).toMatchObject({ x: 1, y: 1 });
		expect(s.tokens['t1'].levelId).toBe('upper');
		expect(s.tokens['t1'].name).toBe('Brannoch');
	});

	it('stay put for an empty path, and an unknown token is ignored', () => {
		let s = withTokens();
		s = fold(s, 6n, tokenMovedKind('t1', []));
		expect(s.tokens['t1'].position).toMatchObject({ x: 2, y: 3 });
		const before = s.tokens;
		s = fold(s, 7n, tokenMovedKind('nobody', [[1, 1]]));
		expect(s.tokens).toEqual(before);
		expect(s.seq).toBe(7n);
	});

	it('are removed by a removal', () => {
		const s = fold(withTokens(), 6n, tokenRemovedKind('t1'));
		expect(Object.keys(s.tokens)).toEqual(['t-gob']);
	});

	it('are replaced by a new Snapshot', () => {
		const s = foldMessage(withTokens(), snapshotMessage(20n, [['only', 'Only']]));
		expect(Object.keys(s.tokens)).toEqual(['only']);
	});
});

describe('creatures', () => {
	const creature = (name: string, hpCurrent = 10) =>
		create(CreatureSchema, { creatureId: 'c1', name, hpCurrent });

	it('are kept from the Snapshot by creature id', () => {
		const s = foldMessage(emptyState(), snapshotMessage(5n, [], [], [creature('Brannoch')]));
		expect(s.creatures['c1'].name).toBe('Brannoch');
	});

	it('are replaced by an upsert with the same id and added with a new one', () => {
		let s = foldMessage(emptyState(), snapshotMessage(5n, [], [], [creature('Brannoch')]));
		s = fold(s, 6n, creatureUpsertedKind(creature('Brannoch Vale', 3)));
		expect(s.creatures['c1'].name).toBe('Brannoch Vale');
		expect(s.creatures['c1'].hpCurrent).toBe(3);
		s = fold(s, 7n, creatureUpsertedKind(create(CreatureSchema, { creatureId: 'c2', name: 'Goblin' })));
		expect(Object.keys(s.creatures).sort()).toEqual(['c1', 'c2']);
	});

	it('are not a log entry', () => {
		const s = fold(joined(), 6n, creatureUpsertedKind(creature('x')));
		expect(s.log).toEqual([]);
	});
});

describe('resolutions', () => {
	const resolution = (state: ResolutionState, over: NonNullable<Parameters<typeof create<typeof ResolutionSchema>>[1]> = {}) =>
		create(ResolutionSchema, {
			resolutionId: 'res-1',
			sourceTokenId: 't1',
			abilityId: 'longsword',
			abilityName: 'Longsword',
			state,
			...over
		});
	const warn = (code: string) => create(WarningSchema, { code, message: 'server words' });

	it('are kept by id with the newest state winning, and appear once in the log', () => {
		let s = joined();
		s = foldMessage(s, abilityEventMessage(6n, resolutionKind(resolution(ResolutionState.TARGETING)), 'intent-1'));
		const done = resolution(ResolutionState.DONE, {
			outcomes: [create(TargetOutcomeSchema, { tokenId: 't-gob', outcome: HitOutcome.HIT })]
		});
		s = foldMessage(s, abilityEventMessage(7n, resolutionKind(done), 'intent-1'));
		expect(s.resolutions['res-1'].state).toBe(ResolutionState.DONE);
		expect(s.resolutions['res-1'].outcomes).toHaveLength(1);
		const results = s.log.filter((e) => e.kind === 'result');
		expect(results).toHaveLength(1);
		expect(results[0].kind === 'result' && results[0].seqs).toEqual([6n, 7n]);
	});

	it('keep their place in the log when updated after later entries', () => {
		let s = joined();
		s = foldMessage(s, abilityEventMessage(6n, resolutionKind(resolution(ResolutionState.TARGETING)), 'i'));
		s = fold(s, 7n, chatKind('mock-gm', 'hi'));
		s = foldMessage(s, abilityEventMessage(8n, resolutionKind(resolution(ResolutionState.DONE)), 'i'));
		expect(s.log.map((e) => e.kind)).toEqual(['result', 'chat']);
	});

	it('carry the intent id and the warnings of their events, each warning once', () => {
		let s = joined();
		s = foldMessage(
			s,
			abilityEventMessage(6n, resolutionKind(resolution(ResolutionState.TARGETING)), 'intent-9', [warn('out_of_range')])
		);
		s = foldMessage(
			s,
			abilityEventMessage(7n, resolutionKind(resolution(ResolutionState.DONE, { warnings: [warn('too_many_targets')] })), '', [
				warn('out_of_range')
			])
		);
		const entry = s.log[0];
		expect(entry.kind === 'result' && entry.clientIntentId).toBe('intent-9');
		expect(entry.kind === 'result' && entry.warnings.map((w) => w.code)).toEqual(['out_of_range', 'too_many_targets']);
	});

	it('are two entries when they are two resolutions', () => {
		let s = joined();
		s = foldMessage(s, abilityEventMessage(6n, resolutionKind(resolution(ResolutionState.DONE)), 'a'));
		s = foldMessage(
			s,
			abilityEventMessage(7n, resolutionKind(resolution(ResolutionState.DONE, { resolutionId: 'res-2' })), 'b')
		);
		expect(s.log).toHaveLength(2);
		expect(Object.keys(s.resolutions).sort()).toEqual(['res-1', 'res-2']);
	});

	it('are marked undone when any event for them is undone, like a roll', () => {
		let s = joined();
		s = foldMessage(s, abilityEventMessage(6n, resolutionKind(resolution(ResolutionState.TARGETING)), 'a'));
		s = foldMessage(s, abilityEventMessage(7n, resolutionKind(resolution(ResolutionState.DONE)), 'a'));
		s = fold(s, 8n, undoneKind(7n));
		expect(s.log[0].undone).toBe(true);
	});

	it('are forgotten with their log entry when the log is trimmed', () => {
		let s = joined();
		s = foldMessage(s, abilityEventMessage(6n, resolutionKind(resolution(ResolutionState.DONE)), 'a'));
		for (let i = 0; i < MAX_LOG_ENTRIES; i++) {
			s = fold(s, 7n + BigInt(i), chatKind('u', `m${i}`));
		}
		expect(s.log.some((e) => e.kind === 'result')).toBe(false);
		expect(s.resolutions['res-1']).toBeUndefined();
	});

	it('are cleared by a new Snapshot', () => {
		let s = foldMessage(joined(), abilityEventMessage(6n, resolutionKind(resolution(ResolutionState.DONE)), 'a'));
		s = foldMessage(s, snapshotMessage(30n));
		expect(s.resolutions).toEqual({});
		expect(s.log).toEqual([]);
	});
});

describe('warnings on a token update', () => {
	const withWarning = (seq: bigint, intentId: string, warnings: string[]) =>
		create(ServerMessageSchema, {
			kind: {
				case: 'event',
				value: create(EventSchema, {
					seq,
					atMs: 1000n + seq,
					clientIntentId: intentId,
					warnings: warnings.map((code) => create(WarningSchema, { code })),
					kind: tokenUpsertedKind('t1', 'Brannoch the Bold')
				})
			}
		});

	it('keeps the server warnings with the intent that caused them', () => {
		const s = foldMessage(joined(), withWarning(6n, 'i-1', ['condition_immune']));
		expect(s.tokenWarnings?.clientIntentId).toBe('i-1');
		expect(s.tokenWarnings?.warnings.map((w) => w.code)).toEqual(['condition_immune']);
	});

	it('has none to begin with, and a plain update leaves the last ones alone', () => {
		expect(joined().tokenWarnings).toBeNull();
		let s = foldMessage(joined(), withWarning(6n, 'i-1', ['condition_immune']));
		s = fold(s, 7n, tokenUpsertedKind('t1', 'Brannoch the Bold'));
		expect(s.tokenWarnings?.clientIntentId).toBe('i-1');
	});
});
