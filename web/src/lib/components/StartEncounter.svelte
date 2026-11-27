<script lang="ts">
	import { getGameClient } from '../net';
	import { rejectionText } from '../chat';
	import {
		buildStartKind,
		creatureIds,
		startBlockedReason,
		startChoices
	} from '../initiative';

	// GM only (the panel shows this only to the GM; the server refuses anyone else). The order
	// appears on the bar when the server's EncounterUpdated arrives: nothing is rolled here.
	const client = getGameClient();
	const { state: gameState, lastRejection } = client;

	const choices = $derived(startChoices($gameState.tokens, $gameState.activeSceneId));
	let ticked = $state<string[]>([]);
	// A token that left the map cannot stay ticked.
	const live = $derived(ticked.filter((id) => choices.some((c) => c.tokenId === id)));
	const blocked = $derived(startBlockedReason(live, $gameState.activeSceneId));
	let sentId = $state<string | null>(null);
	let note = $state('');

	const rejection = $derived(
		$lastRejection && sentId && $lastRejection.clientIntentId === sentId
			? rejectionText($lastRejection.code)
			: ''
	);

	function toggle(id: string) {
		ticked = ticked.includes(id) ? ticked.filter((t) => t !== id) : [...ticked, id];
	}

	function start() {
		const kind = buildStartKind($gameState.activeSceneId, choices, live);
		if (!kind) return;
		sentId = client.send(kind);
		note = sentId ? '' : 'Not connected. Nothing was sent.';
	}
</script>

<form
	class="start"
	onsubmit={(e) => {
		e.preventDefault();
		start();
	}}
>
	<fieldset>
		<legend>Start an encounter</legend>
		{#if choices.length === 0}
			<p class="note">There are no tokens on this map yet.</p>
		{:else}
			<button type="button" onclick={() => (ticked = creatureIds(choices))}>
				Select all creatures
			</button>
			<ul>
				{#each choices as choice (choice.tokenId)}
					<li>
						<label>
							<input
								type="checkbox"
								checked={live.includes(choice.tokenId)}
								onchange={() => toggle(choice.tokenId)}
							/>
							<span>{choice.name}</span>
						</label>
					</li>
				{/each}
			</ul>
		{/if}
	</fieldset>
	<button type="submit" disabled={blocked !== ''} aria-describedby="start-reason">Start encounter</button>
	<p id="start-reason" class="note">{blocked}</p>
	<p class="note" role="status" aria-live="polite">
		{sentId && !rejection ? 'Sent to the server.' : note}
	</p>
	{#if rejection}<p class="error" role="alert">{rejection}</p>{/if}
</form>

<style>
	.start {
		display: flex;
		flex-direction: column;
		gap: 0.6rem;
		align-items: flex-start;
		min-width: 0;
	}
	fieldset {
		margin: 0;
		padding: 0;
		border: 0;
		min-width: 0;
	}
	legend {
		padding: 0;
		font-weight: 600;
	}
	ul {
		display: flex;
		flex-wrap: wrap;
		gap: 0.6rem 0.9rem;
		margin: 0.6rem 0;
		padding: 0;
		list-style: none;
	}
	label {
		display: flex;
		gap: 0.4rem;
		align-items: center;
		min-height: 2rem;
		overflow-wrap: anywhere;
	}
	input[type='checkbox'] {
		width: 1.15rem;
		height: 1.15rem;
	}
	.note {
		margin: 0;
		color: var(--muted);
		min-height: 1.2em;
	}
	.error {
		margin: 0;
		color: #ff9e9e;
	}
</style>
