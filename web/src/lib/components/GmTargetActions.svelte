<script lang="ts">
	import { tick } from 'svelte';
	import { getGameClient } from '../net';
	import { halvePrefill, hpPrefill, type TargetView } from '../chat';
	import ConditionEditor from './ConditionEditor.svelte';
	import HpForm from './HpForm.svelte';

	// The GM's buttons for one target row of a result card. Each opens a form; the card itself
	// never changes HP or conditions: the token updates when the server's event arrives.
	let { target }: { target: TargetView } = $props();

	const { state: gameState } = getGameClient();
	const token = $derived($gameState.tokens[target.tokenId]);
	const halve = $derived(halvePrefill(token, target.appliedTotal));
	const exists = $derived(token !== undefined);

	let open = $state<'hp' | 'halve' | 'conditions' | null>(null);
	let setHpButton = $state<HTMLButtonElement>();
	let halveButton = $state<HTMLButtonElement>();
	let conditionsButton = $state<HTMLButtonElement>();
	// What the HP form starts with, fixed when it opens so the GM's typing is not overwritten.
	let start = $state({ current: '', temp: '', note: '' });

	async function close() {
		const was = open;
		open = null;
		await tick();
		(was === 'hp' ? setHpButton : was === 'halve' ? halveButton : conditionsButton)?.focus();
	}

	function openHp() {
		const p = hpPrefill(token);
		start = { current: p.current, temp: p.temp, note: '' };
		open = 'hp';
	}

	function openHalve() {
		if (!halve.ok) return;
		start = { current: String(halve.current), temp: halve.temp, note: halve.note };
		open = 'halve';
	}
</script>

{#if exists}
	<div class="actions" role="group" aria-label="GM tools for {target.name}">
		<button type="button" bind:this={setHpButton} onclick={openHp}>Set HP</button>
		{#if target.appliedTotal !== null}
			<button
				type="button"
				bind:this={halveButton}
				disabled={!halve.ok}
				aria-describedby={halve.ok ? undefined : `halve-why-${target.tokenId}`}
				onclick={openHalve}>Halve</button
			>
			{#if !halve.ok}<span class="why" id="halve-why-{target.tokenId}">{halve.reason}</span>{/if}
		{/if}
		<button type="button" bind:this={conditionsButton} onclick={() => (open = 'conditions')}>
			Edit conditions
		</button>
	</div>
	{#if open === 'hp' || open === 'halve'}
		<HpForm tokenId={target.tokenId} current={start.current} temp={start.temp} note={start.note} onclose={close} />
	{:else if open === 'conditions'}
		<ConditionEditor tokenId={target.tokenId} onclose={close} />
	{/if}
{/if}

<style>
	.actions {
		display: flex;
		flex-wrap: wrap;
		gap: 0.3rem 0.4rem;
		align-items: center;
		margin-top: 0.3rem;
	}
	.actions button {
		min-height: 1.8rem;
		font-size: 0.85rem;
	}
	.why {
		color: var(--muted);
		font-size: 0.8rem;
	}
</style>
