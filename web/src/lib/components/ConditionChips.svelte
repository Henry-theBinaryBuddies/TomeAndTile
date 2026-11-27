<script lang="ts">
	import type { ConditionList } from '../conditions';

	let { list }: { list: ConditionList } = $props();
	let openId = $state<string | null>(null);
	const open = $derived(list.items.find((i) => i.id === openId));
</script>

{#if !list.known}
	<p class="none">{list.text}</p>
{:else if list.items.length === 0}
	<p class="none">None</p>
{:else}
	<ul class="chips" aria-label="Conditions">
		{#each list.items as item (item.id)}
			<li>
				{#if item.meaning}
					<!-- The meaning opens on tap or Enter, so touch and keyboard get it too. -->
					<button
						type="button"
						class="chip"
						aria-expanded={openId === item.id}
						onclick={() => (openId = openId === item.id ? null : item.id)}>{item.name}</button
					>
				{:else}
					<span class="chip">{item.name}</span>
				{/if}
			</li>
		{/each}
	</ul>
	{#if open}<p class="meaning" role="status">{open.name}: {open.meaning}</p>{/if}
{/if}

<style>
	.chips {
		display: flex;
		flex-wrap: wrap;
		gap: 0.3rem;
		margin: 0;
		padding: 0;
		list-style: none;
	}
	.chip {
		padding: 0.15rem 0.6rem;
		min-height: 1.8rem;
		background: var(--panel-head);
		border: 1px solid var(--border);
		border-radius: 999px;
		color: var(--text);
		font: inherit;
		overflow-wrap: anywhere;
	}
	button.chip {
		cursor: pointer;
	}
	.none,
	.meaning {
		margin: 0.2rem 0 0;
		color: var(--muted);
	}
</style>
