<script lang="ts">
	import { layout } from '../layout/store';
	import { panelsAt, type Edge } from '../layout/model';
	import Panel from './Panel.svelte';

	let { edge }: { edge: Edge } = $props();
	const ids = $derived(panelsAt($layout, edge));
</script>

{#if ids.length > 0}
	<div class="dock dock-{edge}">
		{#each ids as id (id)}
			<Panel {id} mode="docked" />
		{/each}
	</div>
{/if}
