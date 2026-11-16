<script lang="ts">
	import { PANEL_IDS, PANEL_TITLES } from '../layout/model';
	import { phoneOpen } from '../layout/viewport';
	import Panel from './Panel.svelte';

	function toggle(id: (typeof PANEL_IDS)[number]) {
		phoneOpen.update((open) => (open === id ? null : id));
	}
</script>

<svelte:window
	onkeydown={(e) => {
		if (e.key === 'Escape') phoneOpen.set(null);
	}}
/>

{#if $phoneOpen}
	<div class="phone-sheet">
		<Panel id={$phoneOpen} mode="sheet" />
	</div>
{/if}

<nav class="phone-tabs" aria-label="Panels">
	{#each PANEL_IDS as id (id)}
		<button type="button" aria-pressed={$phoneOpen === id} onclick={() => toggle(id)}>
			{PANEL_TITLES[id]}
		</button>
	{/each}
</nav>
