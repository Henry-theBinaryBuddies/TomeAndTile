<script lang="ts">
	import type { ResultCardView } from '../chat';
	import RollCard from './RollCard.svelte';

	let { view, undone }: { view: ResultCardView; undone: boolean } = $props();
</script>

<article class="result" class:undone aria-label={view.announcement}>
	<p class="head">
		<span class="who">{view.sourceName}</span>
		<span class="title">{view.abilityName}</span>
		{#if view.stateText}<span class="tag">{view.stateText}</span>{/if}
		{#if undone}<span class="tag">Undone</span>{/if}
	</p>
	{#if view.targets.length > 0}
		<ol class="targets" aria-label="Targets">
			{#each view.targets as target (target.key)}
				<li class="target">
					<p class="line">
						{#if target.attackNumber !== null}<span class="tag">Attack {target.attackNumber}</span>{/if}
						<span class="name">{target.name}</span>
						<strong class="outcome {target.outcome.kind}">{target.outcome.text}</strong>
					</p>
					{#if target.roll}
						<RollCard who={view.sourceName} view={target.roll} {undone} />
					{:else if target.rollNote}
						<p class="note">{target.rollNote}</p>
					{/if}
					{#each target.damageRolls as damage, i (i)}
						<RollCard who={view.sourceName} view={damage} {undone} />
					{/each}
				</li>
			{/each}
		</ol>
	{/if}
	{#each view.warnings as warning, i (i)}
		<p class="warning">{warning}</p>
	{/each}
</article>

<style>
	.result {
		margin: 0.4rem 0;
		padding: 0.5rem 0.65rem;
		background: var(--panel-head);
		border: 1px solid var(--border);
		border-left: 3px solid var(--accent);
		border-radius: 8px;
		overflow-wrap: anywhere;
	}
	.result.undone {
		opacity: 0.6;
	}
	.head,
	.line {
		display: flex;
		flex-wrap: wrap;
		gap: 0.2rem 0.5rem;
		align-items: baseline;
		margin: 0;
	}
	.who {
		color: var(--accent);
		font-weight: 600;
	}
	.title {
		font-weight: 600;
	}
	.tag {
		padding: 0 0.4rem;
		border: 1px solid var(--border);
		border-radius: 999px;
		color: var(--muted);
		font-size: 0.75rem;
	}
	.targets {
		margin: 0.4rem 0 0;
		padding: 0;
		list-style: none;
		display: flex;
		flex-direction: column;
		gap: 0.4rem;
	}
	.target {
		padding-top: 0.3rem;
		border-top: 1px solid var(--border);
	}
	.outcome.critical {
		color: #ffd27a;
	}
	.outcome.miss {
		color: var(--muted);
	}
	.note {
		margin: 0.2rem 0 0;
		color: var(--muted);
		font-size: 0.85rem;
	}
	.warning {
		margin: 0.35rem 0 0;
		padding: 0.25rem 0.5rem;
		border: 1px solid #8a6d2f;
		border-radius: 6px;
		color: #ffd27a;
		font-size: 0.85rem;
	}
</style>
