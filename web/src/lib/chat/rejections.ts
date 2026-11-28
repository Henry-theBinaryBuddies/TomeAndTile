// Plain-language text for the server's `IntentRejected` codes. The text is
// chosen by the stable code; the server's own message is never shown.

const TEXT: Record<string, string> = {
	empty_chat: 'Type a message first.',
	chat_too_long: 'That message is too long. Shorten it and try again.',
	not_permitted: 'You are not allowed to do that.',
	not_implemented: 'The server does not support that yet.',
	// Setting a map image (ADR 0014).
	unknown_scene: 'That map no longer exists on the server. Reload the page and try again.',
	unknown_level: 'That map level no longer exists on the server. Reload the page and try again.',
	unknown_asset: 'The server no longer has that image. Choose the file again.',
	not_supported_by_mock: 'The test server does not support that.',
	malformed_intent: 'The server could not understand that request.',
	// Dice notation: the mock uses bad_notation, the real server the dice_* codes.
	bad_notation: 'That dice notation is not valid. Try something like d20+5 or 2d6.',
	dice_empty: 'Enter dice to roll, for example d20+5.',
	dice_syntax: 'That dice notation is not valid. Try something like d20+5 or 2d6.',
	dice_bad_sides: 'That die does not exist. Try d4, d6, d8, d10, d12, d20 or d100.',
	dice_bad_count: 'That is not a valid number of dice.',
	dice_too_many_dice: 'That is too many dice in one roll.',
	dice_too_many_terms: 'That roll has too many parts.',
	dice_bad_keep: 'The keep or drop part of that roll is not valid.',
	dice_number_too_large: 'One of the numbers in that roll is too large.',
	dice_too_long: 'That dice notation is too long.',
	// Using an ability (UseAbilityIntent).
	unknown_token: 'That token is not on the map any more.',
	no_creature: 'That token has no character sheet, so it cannot use abilities.',
	unknown_ability: 'That character does not have that ability. Reload the page and try again.',
	no_targets: 'Pick at least one target first.',
	unknown_target: 'One of those targets is gone or cannot be targeted. Pick again.',
	// Encounters.
	no_combatants: 'Tick at least one token to start the fight.',
	encounter_running: 'This map already has a fight running.',
	unknown_encounter: 'That fight is not running any more. Reload the page and try again.',
	unknown_combatant: 'That token is not in the fight.',
	// Setting conditions (SetConditionsIntent).
	bad_condition_id: 'That condition name is not valid.',
	conflicting_conditions: 'The same condition cannot be added and removed at once.',
	too_many_conditions: 'A token cannot have that many conditions.',
	rng_unavailable: 'The server could not roll the dice just now. Try again.'
};

export function rejectionText(code: string): string {
	return TEXT[code] ?? `The server could not do that (${code || 'no reason given'}).`;
}
