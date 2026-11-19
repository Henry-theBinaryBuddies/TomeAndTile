// Loads the renderer's WASM package and starts it on a canvas. The package is built by
// tools/build-renderer.sh into src/lib/renderer/pkg (git-ignored), so it is imported
// only when the map starts, never while the page is being set up or in unit tests.
import type { MapRenderer } from './controller';

export async function startRenderer(canvas: HTMLCanvasElement): Promise<MapRenderer> {
	const mod = await import('../renderer/pkg/tt_renderer.js');
	await mod.default();
	return mod.Renderer.create(canvas);
}
