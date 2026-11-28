<script lang="ts">
	import { untrack } from 'svelte';
	import { getGameClient } from '../net';
	import { buildSetHitPointsKind, parseHp, rejectionText } from '../chat';

	// GM: set a token's hit points. Nothing changes here: the new numbers show on the token when
	// the server's TokenUpserted arrives. The server checks the limits again.
	let {
		tokenId,
		current = '',
		temp = '',
		note = '',
		onclose
	}: { tokenId: string; current?: string; temp?: string; note?: string; onclose: () => void } = $props();

	const client = getGameClient();
	const { state: gameState, lastRejection } = client;

	let currentText = $state(untrack(() => current));
	let tempText = $state(untrack(() => temp));
	let sentId = $state<string | null>(null);
	let sentFrom = $state.raw<unknown>(null);
	let problem = $state('');
	let form = $state<HTMLElement>();

	const token = $derived($gameState.tokens[tokenId]);
	const name = $derived(token?.name.trim() || 'Unknown');
	const rejection = $derived(
		$lastRejection && sentId && $lastRejection.clientIntentId === sentId
			? rejectionText($lastRejection.code)
			: ''
	);
	// The server answers with TokenUpserted, which replaces the token object.
	const saved = $derived(sentId !== null && !rejection && token !== sentFrom);

	function save() {
		const parsed = parseHp(currentText, tempText);
		if (!parsed.ok) {
			problem = parsed.reason;
			return;
		}
		problem = '';
		sentFrom = token;
		sentId = client.send(buildSetHitPointsKind(tokenId, parsed.current, parsed.temp));
		if (!sentId) problem = 'Not connected. Nothing was sent.';
	}

	function onkeydown(e: KeyboardEvent) {
		if (e.key !== 'Escape' || e.isComposing) return;
		e.preventDefault();
		e.stopPropagation();
		onclose();
	}

	$effect(() => {
		form?.querySelector<HTMLInputElement>('input')?.focus();
	});
</script>

<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
<form
	class="hp"
	bind:this={form}
	{onkeydown}
	onsubmit={(e) => {
		e.preventDefault();
		save();
	}}
>
	<p class="legend">Hit points for {name}</p>
	{#if note}<p class="hint">{note}</p>{/if}
	<label>Current <input class="num" inputmode="numeric" autocomplete="off" bind:value={currentText} /></label>
	<label>Temporary <input class="num" inputmode="numeric" autocomplete="off" bind:value={tempText} /></label>
	<div class="buttons">
		<button type="submit">Set hit points</button>
		<button type="button" onclick={onclose}>Cancel</button>
	</div>
	<p class="status" role="status" aria-live="polite">
		{saved ? 'Saved.' : sentId && !rejection ? 'Sent to the server.' : ''}
	</p>
	{#if problem || rejection}
		<p class="error" role="alert">{problem || rejection}</p>
	{/if}
</form>

<style>
	.hp {
		display: flex;
		flex-wrap: wrap;
		gap: 0.3rem 0.8rem;
		align-items: center;
		margin: 0.3rem 0;
		padding: 0.4rem 0.55rem;
		border: 1px solid var(--border);
		border-radius: 8px;
		background: var(--panel-head);
	}
	.legend {
		flex-basis: 100%;
		margin: 0;
		font-weight: 600;
		overflow-wrap: anywhere;
	}
	.hint {
		flex-basis: 100%;
		margin: 0;
		color: var(--muted);
		font-size: 0.9rem;
	}
	label {
		display: flex;
		gap: 0.4rem;
		align-items: center;
	}
	.num {
		width: 4.5rem;
	}
	.buttons {
		display: flex;
		gap: 0.5rem;
	}
	.status {
		flex-basis: 100%;
		margin: 0;
		color: var(--muted);
		min-height: 1.2em;
	}
	.error {
		flex-basis: 100%;
		margin: 0;
		color: #ff9e9e;
	}
</style>
