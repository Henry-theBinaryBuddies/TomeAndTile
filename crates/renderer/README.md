# tt-renderer (version 0)

The map canvas: Rust compiled to WASM, drawing with WebGPU (WebGL2 fallback). It draws the active scene's map image, a square grid, and tokens, and turns token drags into `MoveTokenIntent`s. `web/` owns everything outside the canvas (ADR 0011).

Two halves in one crate:

- **Scene model** (`camera.rs`, `model.rs`, `draw.rs`, `wire.rs`). Pure Rust, no browser, no GPU. Built from decoded `Snapshot`/`Event` messages; also does camera, hit-testing, drag logic and produces the per-frame draw list. Unit-tested natively (`cargo test -p tt-renderer`).
- **Backend and WASM interface** (`gpu.rs`, `web.rs`, `shader.wgsl`). `wasm32` only. One render pipeline, one vertex buffer, a texture per image.

## WASM interface (public API for `web/`)

Generated with `wasm-bindgen --target web`. All numbers are CSS pixels unless stated.

```js
import init, { Renderer, encodeHello } from './pkg/tt_renderer.js';
await init();
const r = await Renderer.create(canvas);          // HTMLCanvasElement; rejects if no GPU backend works
r.setIntentHandler((bytes /* Uint8Array */) => socket.send(bytes));
r.setRejectionHandler((code, message, clientIntentId) => { /* optional */ });
r.resize(cssWidth, cssHeight, devicePixelRatio);  // call on layout changes
socket.send(encodeHello(campaignId, 'dev:<user_id>'));   // first frame; Uint8Array
socket.onmessage = (e) => r.applyServerMessage(new Uint8Array(e.data)); // throws on bad bytes
requestAnimationFrame(function f() { r.render(); requestAnimationFrame(f); });
r.destroy();                                       // removes the canvas listeners
```

| Item | Meaning |
|---|---|
| `Renderer.create(canvas)` | async. Picks WebGPU if the browser has it, otherwise WebGL2. Attaches pointer and wheel listeners to the canvas. |
| `applyServerMessage(bytes)` | One whole `ServerMessage` (one binary WebSocket frame, unchanged). Snapshot replaces all state; events update it; `IntentRejected` clears the matching move preview and calls the rejection handler; other kinds (media token) are ignored. Malformed bytes throw a `JsError` and change nothing. |
| `setIntentHandler(f)` | `f(Uint8Array)` receives one encoded `ClientMessage` per intent. Send it as a binary frame, unchanged. Called outside any internal borrow, so `f` may call back into the renderer. |
| `setRejectionHandler(f)` | `f(code, message, clientIntentId)`. Optional. |
| `resize(w, h, dpr)` | CSS size and device pixel ratio. The backing store becomes `w*dpr` by `h*dpr` (clamped to the GPU's texture limit). The host owns CSS layout and must give the canvas a CSS size that does not depend on its backing store. |
| `render()` | Draw one frame (call from `requestAnimationFrame`). Also uploads images the browser has finished decoding and expires stale move previews. Throws a `JsError` if the GPU backend reports a hard error. |
| `encodeHello(campaignId, sessionToken)` | Returns the `Hello` frame, so the host needs no protobuf library. `campaignId` may be `""` (the server only checks it when both sides set it). |
| `destroy()` | Removes the input listeners. |

**Input.** The renderer attaches its own listeners (`pointerdown/move/up/cancel`, `wheel`, `contextmenu`) to the canvas and uses pointer capture, so `web/` forwards nothing. Only the primary pointer is used; the host should set CSS `touch-action: none` on the canvas. No touch gestures beyond what pointer events give (one finger drags or pans; no pinch).

**Panics.** The model never panics on any input (tests include garbage bytes and non-finite numbers). A panic hook logs to `console.error` anyway.

**Whole `ServerMessage`, not inner bytes.** `web/` passes each frame as received; it never has to decode a `Snapshot` or `Event` itself.

## Event and intent flow

```
socket --Snapshot/Event/IntentRejected--> applyServerMessage --> Model --> draw list --> GPU
canvas pointer events --> Model --(drag released)--> encoded ClientMessage --> intent handler --> socket
```

The model never moves a token itself (ADR 0004). A drag works like this:

1. Pointer down on a token the viewer may move starts a drag: the viewer is the GM, or has role Player and is listed in `controller_user_ids`. Spectators never drag. Pointer down on a token the viewer may not move selects it and pans; pointer down on empty space (or with a non-primary mouse button) clears the selection and pans.
2. After 3 px of travel the token follows the pointer (keeping the grab offset), with a ring showing the snapped cell it would land in.
3. Pointer up snaps the preview to the grid and returns a `ClientMessage { intent { client_intent_id, move_token { token_id, path: [destination], level_id } } }`. A click without movement, or a drop that snaps back to the current position, sends nothing.
4. The token is then drawn at the snapped destination (the "pending preview") while its real position is unchanged.

**Preview rule (the simplest correct one).** A pending preview ends at the first of: any `TokenMoved`, `TokenUpserted` or `TokenRemoved` for that token (the server has answered, so the real position is drawn); an `IntentRejected` whose `client_intent_id` matches (the token snaps back); or 5 s after release (`PENDING_TIMEOUT_MS`). A token with a pending preview cannot be grabbed again. `client_intent_id` is `ci-<random hex per page load>-<counter>`.

## Units and judgement calls

The contract is explicit about units: `Vec2` and `Grid.cell_size` are scene units, which are pixels of the source map image. Where it is silent I chose:

- **Token position is the token's centre** (the mock fixture's `(105, 105)` with a 70-unit grid is the centre of cell (1, 1)).
- **Token size** from `CreatureSize`: Tiny 0.5 cell, Small 1, Medium 1, Large 2, Huge 3, Gargantuan 4, unspecified 1. A token is a disc of that diameter. (The contract does not give Small a smaller size; 1 cell matches the fixture.)
- **Map image** is `Level.background`, drawn with its top-left at scene `(0, 0)` and sized `AssetRef.width` by `height` (or, if those are 0, the decoded image's own size). With no background, there is no map quad and the grid covers the visible area. The grid is clipped to the map when its size is known. A map that has not loaded (or failed) is a flat placeholder rectangle of the same size (reddish on failure).
- **Grid** lines sit at `Grid.offset + k * cell_size`. Only `GRID_KIND_SQUARE` is drawn and snapped to; hex, none and unspecified draw no grid and do not snap. A missing or silly `cell_size` (not finite, under 1 or over 1e6) becomes 70.
- **Snapping.** Odd-size tokens (and Tiny) centre in a cell; even-size tokens (Large, Gargantuan) sit on grid corners, so a 2-cell token covers whole cells. The task text said "cell centre"; centre-in-cell is wrong for even sizes, so this is a deliberate refinement.
- **Active scene and level.** The snapshot's `active_scene_id` (the first scene if it is empty or unknown), and that scene's **first level**. A token is drawn if its `scene_id` matches (or is empty) and its `level_id` matches the level (or is empty). There is no event that changes the active scene (see gaps), so this changes only on a new Snapshot.
- **Draw order** is snapshot/upsert order; later tokens are on top and hit first. The dragged token is drawn last. Hidden tokens (GM view) are drawn at half opacity.
- **Token colour** is a hash of `token_id` (FNV-1a) to a hue. A token image replaces it once loaded; until then, or if loading fails, the coloured disc stays.
- **Grid cost.** Cells under 6 screen px are not drawn; between 6 and 14 px the lines fade in; and a frame never has more than 1500 lines (over that, the grid is skipped).
- **Camera.** Zoom is clamped to 0.02..16 screen px per unit. Wheel zooms about the pointer. The first time a scene and a viewport both exist, the view fits the map (or the tokens plus a margin); any pan or zoom cancels a pending fit.
- **Events apply in arrival order**, whatever their `seq`. The server sends a snapshot then each event once, in order, and a reconnect starts from a fresh snapshot, so `seq` is only recorded. Events of kinds that do not affect the picture are ignored; so are events for unknown tokens, empty paths and non-finite numbers. Coordinates beyond +-1e7 are clamped.
- **Images** are fetched with `fetch`, decoded by the browser with `createImageBitmap`, and copied to a texture with `copy_external_image_to_texture`. Only `http(s)` and relative URLs are fetched; `javascript:`, `data:`, `file:` and the rest are refused. An image bigger than the GPU's maximum texture size counts as failed (no downscaling). No mipmaps, so a big map zoomed far out is aliased.
- **Colour.** A non-sRGB surface format is used when available so colours are drawn as authored.

## Build and run the demo

Needs the wasm target and a `wasm-bindgen-cli` whose version is **exactly** the `wasm-bindgen` version in `Cargo.lock` (currently 0.2.130; the script checks and prints the install command if it differs):

```
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.130 --locked
bash crates/renderer/build-demo.sh          # or: pwsh crates/renderer/build-demo.ps1
```

Add `--dev` (or `-Dev`) for a debug build. Output goes to `crates/renderer/demo/pkg/` (git-ignored).

Run the server with dev sign-in, and serve the demo folder with any static file server (it must send `.wasm` as `application/wasm`; Python's does):

```
TT_DEV_AUTH=1 cargo run -p tt-server                       # ws://localhost:8788/ws, embedded mock seed
python -m http.server 8099 --directory crates/renderer/demo
```

Open `http://localhost:8099/`. Query parameters: `?user=mock-player` (default `mock-gm`), `?ws=ws://host:port/ws`, `?campaign=<id>`. The line above the canvas shows connection state and the last rejection. Drag a token you control; drag empty space or use the right mouse button to pan; wheel to zoom. Dev auth is not security (see `crates/server/README.md`); never enable it on a reachable server.

WebGPU needs a secure context: `localhost` qualifies. On a browser without WebGPU the WebGL2 fallback is used.

## Dependencies

Browser-only crates sit under `[target.'cfg(target_arch = "wasm32")'.dependencies]`, so native `cargo test`/`clippy` do not compile them: `wgpu` 30.0.2 (`default-features = false`, features `std`, `webgpu`, `webgl`, `wgsl`), `wasm-bindgen` 0.2.130, `wasm-bindgen-futures` 0.4.80, `web-sys` 0.3.107 (only the Web API features used), `js-sys` 0.3.107, `bytemuck` 1.25.2 (only `cast_slice` on `f32`; the derive is not used, because it expands to `unsafe impl`, which `unsafe_code = "forbid"` would reject). Native: `prost` and `tt-contracts`. `unsafe_code = "forbid"` applies to this crate unchanged.

## Not done

Hex grids, fog, lighting, walls, rulers, templates, name plates, HP bars, a level switcher, an active-scene change event, pinch zoom and other touch gestures, mipmaps, texture atlasing (one bind group per image), downscaling of oversized images, device-lost recovery beyond reconfiguring the surface, animation of token moves (a confirmed move jumps), token facing, auras and elevation, and wasm size optimisation (`wasm-opt` is not run; the release `.wasm` is about 4.5 MB). Not tested in CI: the GPU path (needs a browser).
