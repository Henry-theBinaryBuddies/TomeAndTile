<script lang="ts">
	import type { HealthView } from '../health';

	let { view }: { view: HealthView } = $props();
</script>

<!-- Said once, in full, for screen readers; the visible text is hidden from them. -->
<span class="health" class:down={view.down}>
	<span class="sr-only">{view.label}</span>
	<span aria-hidden="true">
		{#if view.down}<span class="mark">✕</span>{/if}
		{view.short}
	</span>
</span>

<style>
	.health {
		color: var(--muted);
		font-size: 0.85rem;
		font-weight: 400;
		overflow-wrap: anywhere;
	}
	/* Down is shown by a mark and a dashed border as well as the word, never by colour alone. */
	.health.down {
		padding: 0 0.3rem;
		border: 1px dashed var(--muted);
		border-radius: 4px;
		color: var(--text);
	}
	.mark {
		font-weight: 700;
	}
</style>
