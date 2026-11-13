//! The published WASM interface. Everything `web/` may call lives here; see
//! README.md for the contract. The renderer attaches its own pointer and wheel
//! listeners to the canvas, so the host never forwards input. wasm32 only.

use std::any::Any;
use std::cell::RefCell;
use std::rc::Rc;

use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;
use web_sys::{HtmlCanvasElement, ImageBitmap, PointerEvent, WheelEvent};

use crate::gpu::Gpu;
use crate::model::{Applied, Model};

type Loaded = Vec<(String, Result<ImageBitmap, String>)>;

struct Inner {
    model: Model,
    gpu: Gpu,
    /// Encoded `ClientMessage`s waiting to be handed to the intent handler.
    outbox: Vec<Vec<u8>>,
}

struct Shared {
    inner: RefCell<Inner>,
    canvas: HtmlCanvasElement,
    intent_handler: RefCell<Option<js_sys::Function>>,
    rejection_handler: RefCell<Option<js_sys::Function>>,
    /// Images the browser has finished decoding, waiting for the next frame
    /// to be uploaded to the GPU.
    loaded: Rc<RefCell<Loaded>>,
    listeners: RefCell<Vec<Listener>>,
}

struct Listener {
    name: &'static str,
    function: js_sys::Function,
    _closure: Box<dyn Any>,
}

/// The map canvas. Create with `Renderer.create(canvas)`.
#[wasm_bindgen]
pub struct Renderer {
    shared: Rc<Shared>,
}

/// Build the first frame to send on a new socket (a `ClientMessage` holding a
/// `Hello`), so the host needs no protobuf library. For the dev server use
/// `session_token = "dev:<user_id>"`.
#[wasm_bindgen(js_name = encodeHello)]
pub fn encode_hello_js(campaign_id: &str, session_token: &str) -> Vec<u8> {
    crate::wire::encode_hello(campaign_id, session_token)
}

fn js_err(msg: impl AsRef<str>) -> JsError {
    JsError::new(msg.as_ref())
}

fn now_ms() -> f64 {
    js_sys::Date::now()
}

#[wasm_bindgen]
impl Renderer {
    /// Take over `canvas`: pick WebGPU or WebGL2, build the pipeline and attach
    /// input listeners. Rejects with a message if no GPU backend works.
    /// Call `resize` once afterwards, then `render` each animation frame.
    #[wasm_bindgen(js_name = create)]
    pub async fn create(canvas: HtmlCanvasElement) -> Result<Renderer, JsError> {
        // Without console_error_panic_hook: log panics (there should be none).
        std::panic::set_hook(Box::new(|info| {
            web_sys::console::error_1(&format!("tt-renderer panic: {info}").into());
        }));
        let gpu = Gpu::new(canvas.clone()).await.map_err(js_err)?;
        let nonce = (js_sys::Math::random() * 9_007_199_254_740_992.0) as u64;
        let shared = Rc::new(Shared {
            inner: RefCell::new(Inner {
                model: Model::new(nonce),
                gpu,
                outbox: Vec::new(),
            }),
            canvas,
            intent_handler: RefCell::new(None),
            rejection_handler: RefCell::new(None),
            loaded: Rc::new(RefCell::new(Vec::new())),
            listeners: RefCell::new(Vec::new()),
        });
        attach_listeners(&shared)?;
        Ok(Renderer { shared })
    }

    /// Feed one whole `ServerMessage` (the bytes of one binary WebSocket
    /// frame). Snapshots and events update the scene; a rejection clears the
    /// move preview and calls the rejection handler; other kinds are ignored.
    /// Malformed bytes return an error and change nothing.
    #[wasm_bindgen(js_name = applyServerMessage)]
    pub fn apply_server_message(&self, bytes: &[u8]) -> Result<(), JsError> {
        let applied = {
            let mut inner = self
                .shared
                .inner
                .try_borrow_mut()
                .map_err(|_| js_err("renderer is busy (re-entrant call)"))?;
            inner
                .model
                .apply_server_message(bytes)
                .map_err(|e| js_err(format!("bad ServerMessage: {e}")))?
        };
        if let Applied::Rejected(r) = applied {
            if let Some(f) = self.shared.rejection_handler.borrow().as_ref() {
                let _ = f.call3(
                    &JsValue::NULL,
                    &r.code.into(),
                    &r.message.into(),
                    &r.client_intent_id.into(),
                );
            }
        }
        Ok(())
    }

    /// Register the function that receives each outgoing intent as a
    /// `Uint8Array` holding one encoded `ClientMessage`: send it as a binary
    /// WebSocket frame, unchanged.
    #[wasm_bindgen(js_name = setIntentHandler)]
    pub fn set_intent_handler(&self, f: js_sys::Function) {
        *self.shared.intent_handler.borrow_mut() = Some(f);
    }

    /// Register a function called as `f(code, message, clientIntentId)` when
    /// the server rejects an intent. Optional.
    #[wasm_bindgen(js_name = setRejectionHandler)]
    pub fn set_rejection_handler(&self, f: js_sys::Function) {
        *self.shared.rejection_handler.borrow_mut() = Some(f);
    }

    /// Set the canvas size in CSS pixels and the device pixel ratio. The
    /// backing store becomes `css * dpr` device pixels. The host owns CSS
    /// layout; call this whenever the canvas's CSS size changes.
    pub fn resize(&self, width: f64, height: f64, device_pixel_ratio: f64) -> Result<(), JsError> {
        let mut inner = self
            .shared
            .inner
            .try_borrow_mut()
            .map_err(|_| js_err("renderer is busy (re-entrant call)"))?;
        inner.model.resize(width, height);
        inner.gpu.resize(width, height, device_pixel_ratio);
        Ok(())
    }

    /// Draw one frame. Call from `requestAnimationFrame`.
    pub fn render(&self) -> Result<(), JsError> {
        let result = {
            let mut guard = self
                .shared
                .inner
                .try_borrow_mut()
                .map_err(|_| js_err("renderer is busy (re-entrant call)"))?;
            let inner = &mut *guard;
            // Upload images the browser has decoded since the last frame.
            let loaded = std::mem::take(&mut *self.shared.loaded.borrow_mut());
            for (url, result) in loaded {
                let outcome = match &result {
                    Ok(bitmap) => inner
                        .gpu
                        .upload_image(&url, bitmap)
                        .map(|()| (bitmap.width(), bitmap.height())),
                    Err(e) => Err(e.clone()),
                };
                if let Ok(bitmap) = &result {
                    bitmap.close();
                }
                match outcome {
                    Ok((w, h)) => inner.model.mark_image_loaded(&url, w, h),
                    Err(e) => {
                        web_sys::console::warn_1(&format!("tt-renderer: image {url}: {e}").into());
                        inner.model.mark_image_failed(&url);
                    }
                }
            }
            inner.model.expire_pending(now_ms());
            for url in inner.model.take_image_requests() {
                start_image_load(url, self.shared.loaded.clone());
            }
            let list = inner.model.draw_list();
            inner.gpu.draw(&list)
        };
        flush(&self.shared);
        result.map_err(js_err)
    }

    /// Remove the input listeners. The renderer draws nothing useful after.
    pub fn destroy(&self) {
        for l in self.shared.listeners.borrow_mut().drain(..) {
            let _ = self
                .shared
                .canvas
                .remove_event_listener_with_callback(l.name, &l.function);
        }
    }
}

/// Hand queued intents to the JS handler, outside any borrow of `inner`.
fn flush(shared: &Shared) {
    let out: Vec<Vec<u8>> = match shared.inner.try_borrow_mut() {
        Ok(mut inner) => std::mem::take(&mut inner.outbox),
        Err(_) => return,
    };
    if out.is_empty() {
        return;
    }
    let handler = shared.intent_handler.borrow().clone();
    let Some(f) = handler else { return };
    for bytes in out {
        let array = js_sys::Uint8Array::from(bytes.as_slice());
        if let Err(e) = f.call1(&JsValue::NULL, &array) {
            web_sys::console::error_2(&"tt-renderer: intent handler threw".into(), &e);
        }
    }
}

fn start_image_load(url: String, loaded: Rc<RefCell<Loaded>>) {
    wasm_bindgen_futures::spawn_local(async move {
        let result = load_bitmap(&url).await.map_err(|e| {
            e.as_string()
                .or_else(|| js_sys::JSON::stringify(&e).ok().and_then(|s| s.as_string()))
                .unwrap_or_else(|| "load failed".into())
        });
        loaded.borrow_mut().push((url, result));
    });
}

/// fetch -> Blob -> createImageBitmap. The browser decodes; no image crate.
async fn load_bitmap(url: &str) -> Result<ImageBitmap, JsValue> {
    let window = web_sys::window().ok_or_else(|| JsValue::from_str("no window"))?;
    let resp: web_sys::Response = JsFuture::from(window.fetch_with_str(url))
        .await?
        .dyn_into()?;
    if !resp.ok() {
        return Err(JsValue::from_str(&format!("HTTP {}", resp.status())));
    }
    let blob: web_sys::Blob = JsFuture::from(resp.blob()?).await?.dyn_into()?;
    let bitmap: ImageBitmap = JsFuture::from(window.create_image_bitmap_with_blob(&blob)?)
        .await?
        .dyn_into()?;
    Ok(bitmap)
}

// ------------------------------------------------------------------- input

fn listen<E: wasm_bindgen::convert::FromWasmAbi + 'static>(
    shared: &Rc<Shared>,
    name: &'static str,
    f: impl FnMut(E) + 'static,
) -> Result<(), JsError> {
    let closure = Closure::wrap(Box::new(f) as Box<dyn FnMut(E)>);
    let function: js_sys::Function = closure.as_ref().unchecked_ref::<js_sys::Function>().clone();
    shared
        .canvas
        .add_event_listener_with_callback(name, &function)
        .map_err(|_| js_err(format!("could not add {name} listener")))?;
    shared.listeners.borrow_mut().push(Listener {
        name,
        function,
        _closure: Box::new(closure),
    });
    Ok(())
}

/// Pointer position in CSS pixels relative to the canvas.
fn local(canvas: &HtmlCanvasElement, x: i32, y: i32) -> (f64, f64) {
    let r = canvas.get_bounding_client_rect();
    (f64::from(x) - r.left(), f64::from(y) - r.top())
}

fn attach_listeners(shared: &Rc<Shared>) -> Result<(), JsError> {
    // Run `f` on the model, then flush any intents it queued. A re-entrant
    // call (the handler calling back into us) is dropped, never a panic.
    fn with_model(shared: &Shared, f: impl FnOnce(&mut Model, &mut Vec<Vec<u8>>)) {
        if let Ok(mut inner) = shared.inner.try_borrow_mut() {
            let inner = &mut *inner;
            f(&mut inner.model, &mut inner.outbox);
        }
        flush(shared);
    }

    let s = shared.clone();
    listen(shared, "pointerdown", move |e: PointerEvent| {
        if !e.is_primary() {
            return;
        }
        e.prevent_default();
        let _ = s.canvas.set_pointer_capture(e.pointer_id());
        let (x, y) = local(&s.canvas, e.client_x(), e.client_y());
        with_model(&s, |m, _| m.pointer_down(x, y, e.button()));
    })?;

    let s = shared.clone();
    listen(shared, "pointermove", move |e: PointerEvent| {
        if !e.is_primary() {
            return;
        }
        let (x, y) = local(&s.canvas, e.client_x(), e.client_y());
        with_model(&s, |m, _| m.pointer_move(x, y));
    })?;

    let s = shared.clone();
    listen(shared, "pointerup", move |e: PointerEvent| {
        if !e.is_primary() {
            return;
        }
        let _ = s.canvas.release_pointer_capture(e.pointer_id());
        let (x, y) = local(&s.canvas, e.client_x(), e.client_y());
        with_model(&s, |m, out| {
            if let Some(bytes) = m.pointer_up(x, y, now_ms()) {
                out.push(bytes);
            }
        });
    })?;

    let s = shared.clone();
    listen(shared, "pointercancel", move |_: PointerEvent| {
        with_model(&s, |m, _| m.pointer_cancel());
    })?;

    let s = shared.clone();
    listen(shared, "wheel", move |e: WheelEvent| {
        e.prevent_default();
        let (x, y) = local(&s.canvas, e.client_x(), e.client_y());
        // Normalise to pixels: 0 = pixel, 1 = line, 2 = page.
        let delta = match e.delta_mode() {
            1 => e.delta_y() * 16.0,
            2 => e.delta_y() * s.canvas.client_height() as f64,
            _ => e.delta_y(),
        };
        with_model(&s, |m, _| m.wheel(x, y, delta));
    })?;

    // Right button pans; suppress the browser menu.
    listen(shared, "contextmenu", move |e: web_sys::Event| {
        e.prevent_default();
    })?;
    Ok(())
}
