// Display names for condition ids. The ids are the server's (scene.proto): the 15 SRD conditions
// plus any label matching [a-z0-9_]{1,40}. This table holds a name and a one-line meaning only,
// no rules text: the server applies advantage, disadvantage and auto-fail, never the UI.

export const SRD_CONDITIONS = [
	'blinded',
	'charmed',
	'deafened',
	'exhaustion',
	'frightened',
	'grappled',
	'incapacitated',
	'invisible',
	'paralyzed',
	'petrified',
	'poisoned',
	'prone',
	'restrained',
	'stunned',
	'unconscious'
] as const;

const TABLE: Record<string, { name: string; meaning: string }> = {
	blinded: { name: 'Blinded', meaning: 'Cannot see.' },
	charmed: { name: 'Charmed', meaning: 'Under a charm effect.' },
	deafened: { name: 'Deafened', meaning: 'Cannot hear.' },
	exhaustion: { name: 'Exhaustion', meaning: 'Worn down. The GM tracks the level.' },
	frightened: { name: 'Frightened', meaning: 'Afraid of something.' },
	grappled: { name: 'Grappled', meaning: 'Held in place.' },
	incapacitated: { name: 'Incapacitated', meaning: 'Cannot take actions.' },
	invisible: { name: 'Invisible', meaning: 'Cannot be seen.' },
	paralyzed: { name: 'Paralyzed', meaning: 'Cannot move or act.' },
	petrified: { name: 'Petrified', meaning: 'Turned to stone.' },
	poisoned: { name: 'Poisoned', meaning: 'Affected by poison.' },
	prone: { name: 'Prone', meaning: 'Lying on the ground.' },
	restrained: { name: 'Restrained', meaning: 'Held so it cannot move.' },
	stunned: { name: 'Stunned', meaning: 'Dazed and unable to act.' },
	unconscious: { name: 'Unconscious', meaning: 'Out cold.' }
};

export interface ConditionName {
	id: string;
	/** The display name, or the id exactly as sent when it is not in the table. */
	name: string;
	/** One line, or "" for an id the table does not know. */
	meaning: string;
	srd: boolean;
}

/** Own-property check, so an id like "constructor" is never taken from the prototype. */
function known(id: string): boolean {
	return Object.prototype.hasOwnProperty.call(TABLE, id);
}

/** An unknown id is shown as sent, as text, and never dropped. */
export function conditionName(id: string): ConditionName {
	return known(id)
		? { id, ...TABLE[id], srd: true }
		: { id, name: id, meaning: '', srd: false };
}
