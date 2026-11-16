<!--
	Full-window element that will hold the map canvas. The renderer (crates/renderer)
	is wired in by a later issue: it takes over a <canvas> placed inside the slot.
	Nothing is drawn here, and web/ never draws on the canvas (ADR 0011).
-->
<main id="map" class="map" aria-label="Map">
	<div id="map-canvas-slot" class="map-slot" data-map-canvas-slot></div>
	<p class="map-hint">The map will appear here.</p>
</main>
