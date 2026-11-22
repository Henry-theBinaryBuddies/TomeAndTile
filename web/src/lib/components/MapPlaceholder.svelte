<!--
	Full-window map. The canvas in the slot belongs to the renderer (crates/renderer): it is
	handed the canvas, receives every server frame, and sends token moves back through the
	socket client. Nothing in web/ draws on the canvas or reads its pixels (ADR 0011).
	The file keeps its old name so the shell's import stays the same.
-->
<script lang="ts">
	import { onMount } from 'svelte';
	import { rejectionText } from '../chat';
	import { createMapController, startRenderer } from '../map';
	import { getGameClient } from '../net';
	import { endTargetingOnEscape, mapPickingFor, pickOnMap, targetingSession } from '../sheet';

	let canvas: HTMLCanvasElement | undefined = $state();
	let slot: HTMLDivElement | undefined = $state();
	let ready = $state(false);
	// Why the map is not running, or null.
	let unavailable = $state<string | null>(null);
	// The last refusal of a token move, shown for a few seconds.
	let refusal = $state<string | null>(null);
	let refusalTimer: ReturnType<typeof setTimeout> | null = null;
	// Picking targets for an ability from the Sheet: tokens can be clicked on the map too.
	const picking = $derived(mapPickingFor($targetingSession).enabled);

	function showRefusal(code: string): void {
		showNotice(rejectionText(code));
	}

	// One short notice at a time, in the page's existing status region.
	function showNotice(text: string): void {
		refusal = text;
		if (refusalTimer) clearTimeout(refusalTimer);
		refusalTimer = setTimeout(() => {
			refusal = null;
		}, 6000);
	}

	onMount(() => {
		const target = canvas;
		const box = slot;
		if (!target || !box) return;
		// Subscribes to the socket's frames now. The Shell opens the connection after
		// its children mount, so the first Snapshot is never missed.
		const controller = createMapController({
			socket: getGameClient(),
			createRenderer: () => startRenderer(target),
			scheduler: {
				request: (callback) => requestAnimationFrame(callback),
				cancel: (handle) => cancelAnimationFrame(handle)
			},
			size: () => {
				const rect = box.getBoundingClientRect();
				return { width: rect.width, height: rect.height, devicePixelRatio: window.devicePixelRatio || 1 };
			},
			onReady: () => {
				ready = true;
			},
			onUnavailable: (message) => {
				ready = false;
				unavailable = message;
			},
			onRejection: showRefusal,
			onFrameProblem: showNotice,
			onPick: pickOnMap
		});
		// Follows the Sheet's picks: picking on or off, and which tokens are ringed.
		const stopPicking = targetingSession.subscribe((session) => {
			const { enabled, highlights } = mapPickingFor(session);
			controller.setPicking(enabled);
			controller.setHighlights(highlights);
		});
		const observer = new ResizeObserver(() => controller.resize());
		observer.observe(box);
		// Browser zoom changes the pixel ratio without always changing the box size.
		const onWindowResize = () => controller.resize();
		window.addEventListener('resize', onWindowResize);
		return () => {
			stopPicking();
			window.removeEventListener('resize', onWindowResize);
			observer.disconnect();
			if (refusalTimer) clearTimeout(refusalTimer);
			controller.destroy();
		};
	});
</script>

<!-- Escape ends target picking even while the Sheet is closed (it is, on a phone). -->
<svelte:window onkeydowncapture={endTargetingOnEscape} />

<main id="map" class="map" aria-label="Map">
	<div id="map-canvas-slot" class="map-slot" data-map-canvas-slot bind:this={slot}>
		<canvas bind:this={canvas} aria-hidden="true"></canvas>
	</div>
	{#if unavailable}
		<p class="map-hint map-unavailable" role="status">{unavailable}</p>
	{:else if !ready}
		<p class="map-hint" role="status">Loading the map…</p>
	{/if}
</main>

<!--
	Outside <main>: a fixed element is its own stacking layer, so inside it the notice could
	never rise above the docked panels. The region stays in the page so screen readers
	announce the text when it appears.
-->
<div class="map-refusal-region" role="status">
	{#if picking}
		<p class="map-refusal map-picking">
			{ready
				? 'Picking targets: click or tap a token on the map, or use the list in the Sheet. Confirm or cancel in the Sheet, or press Escape.'
				: 'Picking targets: use the list in the Sheet. Confirm or cancel in the Sheet, or press Escape.'}
		</p>
	{/if}
	{#if refusal}
		<p class="map-refusal">{refusal}</p>
	{/if}
</div>

<style>
	/* The picking notice and a refusal can both show: keep a gap between them. */
	.map-refusal-region .map-refusal + .map-refusal {
		margin-top: 0.4rem;
	}
</style>
