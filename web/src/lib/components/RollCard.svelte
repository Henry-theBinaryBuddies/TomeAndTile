<script lang="ts">
	import { rollAnnouncement, type RollCardView } from '../chat';

	let { who, view, undone }: { who: string; view: RollCardView; undone: boolean } = $props();
</script>

<article class="roll" class:undone class:critical={view.critical} class:fumble={view.fumble}>
	<p class="sr-only">{rollAnnouncement(who, view)}{undone ? ' (undone)' : ''}</p>
	<div aria-hidden="true">
		<p class="head">
			<span class="who">{who}</span>
			<span class="title">{view.title}</span>
			{#if view.modeText}<span class="tag">{view.modeText}</span>{/if}
			{#if view.hidden}<span class="tag">GM only</span>{/if}
			{#if undone}<span class="tag">Undone</span>{/if}
		</p>
		{#if view.notation}<p class="notation">{view.notation}</p>{/if}
		<ul class="dice">
			{#each view.dice as die, i (i)}
				<li class:dropped={die.dropped}>{die.text}</li>
			{/each}
			{#each view.modifiers as m, i (i)}
				<li class="mod">{m.value}{m.source ? ` ${m.source}` : ''}</li>
			{/each}
		</ul>
		<p class="total">
			Total <strong>{view.total}</strong>
			{#if view.critical}<span class="flag">Critical</span>{/if}
			{#if view.fumble}<span class="flag">Fumble</span>{/if}
		</p>
	</div>
</article>

<style>
	.roll {
		margin: 0.4rem 0;
		padding: 0.5rem 0.65rem;
		background: var(--panel-head);
		border: 1px solid var(--border);
		border-radius: 8px;
	}
	.roll.critical {
		border-color: #6fd08c;
	}
	.roll.fumble {
		border-color: #ff8a8a;
	}
	.roll.undone {
		opacity: 0.6;
	}
	p {
		margin: 0;
	}
	.head {
		display: flex;
		flex-wrap: wrap;
		gap: 0.25rem 0.5rem;
		align-items: baseline;
	}
	.who {
		color: var(--muted);
	}
	.title {
		font-weight: 600;
		overflow-wrap: anywhere;
	}
	.tag {
		padding: 0 0.4rem;
		font-size: 0.75rem;
		border: 1px solid var(--border);
		border-radius: 999px;
		color: var(--muted);
	}
	.notation {
		color: var(--muted);
		font-size: 0.85rem;
	}
	.dice {
		display: flex;
		flex-wrap: wrap;
		gap: 0.25rem 0.75rem;
		margin: 0.3rem 0;
		padding: 0;
		list-style: none;
	}
	.dropped {
		color: var(--muted);
		text-decoration: line-through;
	}
	.total strong {
		font-size: 1.2rem;
	}
	.flag {
		margin-left: 0.5rem;
		font-weight: 600;
	}
</style>
