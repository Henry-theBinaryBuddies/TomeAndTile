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

	let canvas: HTMLCanvasElement | undefined = $state();
	let slot: HTMLDivElement | undefined = $state();
	let ready = $state(false);
	// Why the map is not running, or null.
	let unavailable = $state<string | null>(null);
	// The last refusal of a token move, shown for a few seconds.
	let refusal = $state<string | null>(null);
	let refusalTimer: ReturnType<typeof setTimeout> | null = null;

	function showRefusal(code: string): void {
		refusal = rejectionText(code);
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
			onRejection: showRefusal
		});
		const observer = new ResizeObserver(() => controller.resize());
		observer.observe(box);
		// Browser zoom changes the pixel ratio without always changing the box size.
		const onWindowResize = () => controller.resize();
		window.addEventListener('resize', onWindowResize);
		return () => {
			window.removeEventListener('resize', onWindowResize);
			observer.disconnect();
			if (refusalTimer) clearTimeout(refusalTimer);
			controller.destroy();
		};
	});
</script>

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
	{#if refusal}
		<p class="map-refusal">{refusal}</p>
	{/if}
</div>
