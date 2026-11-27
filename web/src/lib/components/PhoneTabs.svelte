<script lang="ts">
	import { PANEL_IDS, PANEL_TITLES } from '../layout/model';
	import { isSheetOpen, nextOpen, phoneOpen } from '../layout/viewport';
	import Panel from './Panel.svelte';

	function toggle(id: (typeof PANEL_IDS)[number]) {
		phoneOpen.update((open) => nextOpen(open, id));
	}
</script>

<svelte:window
	onkeydown={(e) => {
		if (e.key === 'Escape') phoneOpen.set(null);
	}}
/>

<!-- Every panel stays mounted and only the open one is shown, so switching tabs keeps a half-typed
     message, a ticked checklist, an open form and the targeting list (#113). -->
{#each PANEL_IDS as id (id)}
	<div class="phone-sheet" class:closed={!isSheetOpen($phoneOpen, id)}>
		<Panel {id} mode="sheet" />
	</div>
{/each}

<nav class="phone-tabs" aria-label="Panels">
	{#each PANEL_IDS as id (id)}
		<button type="button" aria-pressed={isSheetOpen($phoneOpen, id)} onclick={() => toggle(id)}>
			{PANEL_TITLES[id]}
		</button>
	{/each}
</nav>
