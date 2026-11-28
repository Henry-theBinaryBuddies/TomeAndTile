<script lang="ts">
	import { tick } from 'svelte';
	import { getGameClient } from '../net';
	import { rejectionText } from '../chat';
	import {
		buildRemoveTokenKind,
		removableTokens,
		removeQuestion,
		type RemovableToken
	} from '../gmtools';

	// Takes a token off the map after a question that names it. The token leaves the list when
	// the server's TokenRemoved arrives.
	const client = getGameClient();
	const { state: gameState, lastRejection } = client;

	const tokens = $derived(removableTokens($gameState.tokens, $gameState.activeSceneId));
	let asking = $state<RemovableToken | null>(null);
	let sentIds = $state<string[]>([]);
	let note = $state('');
	const rejection = $derived(
		$lastRejection && sentIds.includes($lastRejection.clientIntentId)
			? rejectionText($lastRejection.code)
			: ''
	);

	async function stopAsking(tokenId: string) {
		asking = null;
		await tick();
		document.querySelector<HTMLButtonElement>(`[data-remove-token="${CSS.escape(tokenId)}"]`)?.focus();
	}

	// The question replaces the button, so focus goes to its safe answer ("Keep").
	async function ask(token: RemovableToken) {
		asking = token;
		await tick();
		document.querySelector<HTMLButtonElement>('.remove [role="group"] button:last-of-type')?.focus();
	}

	function remove(token: RemovableToken) {
		const id = client.send(buildRemoveTokenKind(token.tokenId));
		if (id) {
			sentIds = [...sentIds, id];
			note = 'Sent to the server.';
		} else note = 'Not connected. Nothing was sent.';
		asking = null;
	}

	function onkeydown(e: KeyboardEvent, tokenId: string) {
		if (e.key === 'Escape' && !e.isComposing) {
			e.preventDefault();
			e.stopPropagation();
			stopAsking(tokenId);
		}
	}
</script>

<section class="remove" aria-labelledby="remove-title">
	<h3 id="remove-title">Tokens on this map</h3>
	{#if tokens.length === 0}
		<p class="note">There are no tokens on this map.</p>
	{:else}
		<ul aria-label="Tokens on this map">
			{#each tokens as token (token.tokenId)}
				<li>
					<span class="name">{token.name}</span>
					{#if asking?.tokenId === token.tokenId}
						<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
						<span role="group" aria-label={removeQuestion(token.name)} onkeydown={(e) => onkeydown(e, token.tokenId)}>
							{removeQuestion(token.name)}
							<button type="button" onclick={() => remove(token)}>Yes, remove</button>
							<button type="button" onclick={() => stopAsking(token.tokenId)}>Keep</button>
						</span>
					{:else}
						<button
							type="button"
							data-remove-token={token.tokenId}
							aria-label="Remove {token.name}"
							onclick={() => ask(token)}>Remove</button
						>
					{/if}
				</li>
			{/each}
		</ul>
	{/if}
	<p class="note" role="status" aria-live="polite">{rejection ? '' : note}</p>
	{#if rejection}<p class="error" role="alert">{rejection}</p>{/if}
</section>

<style>
	.remove {
		display: flex;
		flex-direction: column;
		gap: 0.3rem;
	}
	h3 {
		margin: 0;
		font-size: 1rem;
	}
	ul {
		display: flex;
		flex-direction: column;
		gap: 0.25rem;
		margin: 0;
		padding: 0;
		list-style: none;
	}
	li {
		display: flex;
		flex-wrap: wrap;
		gap: 0.3rem 0.6rem;
		align-items: center;
	}
	.name {
		overflow-wrap: anywhere;
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
