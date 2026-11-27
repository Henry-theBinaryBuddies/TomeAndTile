<script lang="ts">
	import { untrack } from 'svelte';
	import { getGameClient } from '../net';
	import { rejectionText, warningText } from '../chat';
	import {
		buildSetConditionsKind,
		closesEditor,
		editorOptions,
		immuneNote,
		toggleCondition
	} from '../conditions';

	// GM only: the parent shows the button that opens this only when the role is GM, and the
	// server refuses anyone else (not_permitted). Nothing here changes a condition on screen:
	// the list updates when the server's TokenUpserted arrives.
	let { tokenId, onclose }: { tokenId: string; onclose: () => void } = $props();

	const client = getGameClient();
	const { state: gameState, lastRejection } = client;

	const token = $derived($gameState.tokens[tokenId]);
	const creature = $derived(token ? $gameState.creatures[token.creatureId] : undefined);
	const name = $derived(token?.name.trim() || 'Unknown');
	// What the token had when the editor opened: the boxes start from it.
	const startedWith = untrack(() => [...($gameState.tokens[tokenId]?.conditions ?? [])]);
	let selected = $state<string[]>([...startedWith]);
	const options = $derived(editorOptions(startedWith));

	let sentId = $state<string | null>(null);
	let sentFrom = $state.raw<unknown>(null);
	let note = $state('');
	let form = $state<HTMLElement>();

	const rejection = $derived(
		$lastRejection && sentId && $lastRejection.clientIntentId === sentId
			? rejectionText($lastRejection.code)
			: ''
	);
	const serverWarnings = $derived(
		sentId && $gameState.tokenWarnings?.clientIntentId === sentId
			? $gameState.tokenWarnings.warnings.map((w) => warningText(w.code))
			: []
	);
	// The server answers a change with TokenUpserted, which replaces the token object.
	const saved = $derived(sentId !== null && !rejection && token !== sentFrom);

	function save() {
		const kind = buildSetConditionsKind(tokenId, startedWith, selected);
		if (!kind) {
			note = 'No changes to send.';
			return;
		}
		note = '';
		sentFrom = token;
		sentId = client.send(kind);
		if (!sentId) note = 'Not connected. Nothing was sent.';
	}

	function onkeydown(e: KeyboardEvent) {
		if (!closesEditor(e)) return;
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
	class="editor"
	bind:this={form}
	{onkeydown}
	onsubmit={(e) => {
		e.preventDefault();
		save();
	}}
>
	<fieldset>
		<legend>Conditions for {name}</legend>
		<ul>
			{#each options as option (option.id)}
				{@const immune = immuneNote(option.id, creature)}
				<li>
					<label>
						<input
							type="checkbox"
							checked={selected.includes(option.id)}
							onchange={() => (selected = toggleCondition(selected, option.id))}
						/>
						<span>{option.name}</span>
					</label>
					{#if immune}<span class="immune">{immune}</span>{/if}
				</li>
			{/each}
		</ul>
	</fieldset>
	<div class="buttons">
		<button type="submit">Save</button>
		<button type="button" onclick={onclose}>Cancel</button>
	</div>
	<p class="status" role="status" aria-live="polite">
		{saved ? 'Saved.' : sentId && !rejection ? 'Sent to the server.' : note}
	</p>
	{#if rejection || serverWarnings.length > 0}
		<div role="alert">
			{#if rejection}<p class="error">{rejection}</p>{/if}
			{#each serverWarnings as w, i (i)}<p class="error">{w}</p>{/each}
		</div>
	{/if}
</form>

<style>
	.editor {
		margin: 0.4rem 0;
		padding: 0.5rem 0.65rem;
		border: 1px solid var(--border);
		border-radius: 8px;
		background: var(--panel-head);
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
		overflow-wrap: anywhere;
	}
	ul {
		display: grid;
		grid-template-columns: repeat(auto-fill, minmax(9rem, 1fr));
		gap: 0.6rem;
		margin: 0.6rem 0;
		padding: 0;
		list-style: none;
	}
	label {
		display: flex;
		gap: 0.4rem;
		align-items: center;
		min-height: 2rem;
	}
	input[type='checkbox'] {
		width: 1.15rem;
		height: 1.15rem;
	}
	.immune {
		display: block;
		color: var(--muted);
		font-size: 0.8rem;
	}
	.buttons {
		display: flex;
		gap: 0.5rem;
	}
	.status {
		margin: 0.3rem 0 0;
		color: var(--muted);
		min-height: 1.2em;
	}
	.error {
		margin: 0.2rem 0 0;
		color: #ff9e9e;
	}
</style>
