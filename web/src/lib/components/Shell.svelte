<script lang="ts">
	import { onMount } from 'svelte';
	import { getGameClient } from '../net';
	import { layout } from '../layout/store';
	import { floatingPanels } from '../layout/model';
	import { isPhone, viewportSize } from '../layout/viewport';
	import DockRegion from './DockRegion.svelte';
	import MapPlaceholder from './MapPlaceholder.svelte';
	import Panel from './Panel.svelte';
	import PhoneTabs from './PhoneTabs.svelte';

	const floating = $derived(floatingPanels($layout));

	// One connection for the whole page, opened here so panels see messages from the start.
	onMount(() => {
		getGameClient().connect();
	});

	// Keep floating panels on screen when the window shrinks.
	$effect(() => {
		layout.clamp($viewportSize);
	});
</script>

<h1 class="sr-only">Tome &amp; Tile</h1>

<MapPlaceholder />

{#if $isPhone}
	<PhoneTabs />
{:else}
	<div class="shell">
		<DockRegion edge="top" />
		<DockRegion edge="left" />
		<DockRegion edge="right" />
		<DockRegion edge="bottom" />
	</div>
	{#each floating as id (id)}
		<Panel {id} mode="floating" />
	{/each}
{/if}
