use std::cell::RefCell;
use std::rc::Rc;

use shooter_shared::protocol::{decode_s2c, encode_c2s, C2s, Input, S2c};
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::{CanvasRenderingContext2d, HtmlCanvasElement, KeyboardEvent, MouseEvent, WebSocket};

thread_local! {
    static APP: RefCell<Option<Rc<App>>> = const { RefCell::new(None) };
}

struct App {
    ws: WebSocket,
    ctx: CanvasRenderingContext2d,
    canvas: HtmlCanvasElement,
    hud: web_sys::HtmlElement,

    seq: RefCell<u32>,
    input: RefCell<InputState>,
    me: RefCell<Option<u32>>,
    last_snapshot: RefCell<Option<shooter_shared::protocol::Snapshot>>,
}

#[derive(Clone, Copy, Default)]
struct InputState {
    up: bool,
    down: bool,
    left: bool,
    right: bool,
    shoot: bool,
    aim_x: i16,
    aim_y: i16,
}

#[wasm_bindgen(start)]
pub fn start() -> Result<(), JsValue> {
    console_error_panic_hook::set_once();

    let window = web_sys::window().unwrap();
    let document = window.document().unwrap();

    let canvas: HtmlCanvasElement = document
        .get_element_by_id("game")
        .unwrap()
        .dyn_into()?;

    let hud: web_sys::HtmlElement = document
        .get_element_by_id("hud")
        .unwrap()
        .dyn_into()?;

    let ctx: CanvasRenderingContext2d = canvas
        .get_context("2d")?
        .unwrap()
        .dyn_into()?;

    // Size to CSS pixels; we redraw on every frame so keep it simple.
    resize_canvas(&window, &canvas);

    // Connect
    let loc = window.location();
    let host = loc.host()?;
    let ws_url = format!("ws://{host}/ws");
    let ws = WebSocket::new(&ws_url)?;
    ws.set_binary_type(web_sys::BinaryType::Arraybuffer);

    let app = Rc::new(App {
        ws: ws.clone(),
        ctx,
        canvas: canvas.clone(),
        hud,
        seq: RefCell::new(0),
        input: RefCell::new(InputState::default()),
        me: RefCell::new(None),
        last_snapshot: RefCell::new(None),
    });

    // WS message handler
    {
        let app = app.clone();
        let onmessage = Closure::<dyn FnMut(_)>::new(move |ev: web_sys::MessageEvent| {
            if let Ok(buf) = ev.data().dyn_into::<js_sys::ArrayBuffer>() {
                let bytes = js_sys::Uint8Array::new(&buf).to_vec();
                if let Ok(msg) = decode_s2c(&bytes) {
                    match msg {
                        S2c::Welcome { client_id } => {
                            *app.me.borrow_mut() = Some(client_id);
                            app.hud.set_inner_text(&format!("Connected as #{client_id}"));
                        }
                        S2c::Snapshot(s) => {
                            *app.last_snapshot.borrow_mut() = Some(s);
                        }
                    }
                }
            }
        });
        ws.set_onmessage(Some(onmessage.as_ref().unchecked_ref()));
        onmessage.forget();
    }

    // Input
    install_input_handlers(&window, &document, app.clone())?;

    // Render loop
    {
        let window = window.clone();
        let f = Rc::new(RefCell::new(None));
        let g = f.clone();
        let app2 = app.clone();

        *g.borrow_mut() = Some(Closure::<dyn FnMut()>::new(move || {
            resize_canvas(&window, &app2.canvas);
            draw(&app2);

            // Send input at ~60fps.
            send_input(&app2);

            window
                .request_animation_frame(
                    f.borrow().as_ref().unwrap().as_ref().unchecked_ref(),
                )
                .unwrap();
        }));

        window.request_animation_frame(g.borrow().as_ref().unwrap().as_ref().unchecked_ref())?;
        g.borrow().as_ref().unwrap().forget();
    }

    APP.with(|a| *a.borrow_mut() = Some(app));
    Ok(())
}

fn resize_canvas(window: &web_sys::Window, canvas: &HtmlCanvasElement) {
    let dpr = window.device_pixel_ratio();
    let w = (window.inner_width().ok().and_then(|v| v.as_f64()).unwrap_or(800.0) * dpr) as u32;
    let h = (window.inner_height().ok().and_then(|v| v.as_f64()).unwrap_or(600.0) * dpr) as u32;
    if canvas.width() != w || canvas.height() != h {
        canvas.set_width(w);
        canvas.set_height(h);
    }
}

fn install_input_handlers(
    window: &web_sys::Window,
    document: &web_sys::Document,
    app: Rc<App>,
) -> Result<(), JsValue> {
    // Keyboard
    {
        let app = app.clone();
        let onkeydown = Closure::<dyn FnMut(_)>::new(move |e: KeyboardEvent| {
            let mut inp = app.input.borrow_mut();
            match e.key().as_str() {
                "w" | "W" => inp.up = true,
                "s" | "S" => inp.down = true,
                "a" | "A" => inp.left = true,
                "d" | "D" => inp.right = true,
                _ => {}
            }
        });
        document.add_event_listener_with_callback("keydown", onkeydown.as_ref().unchecked_ref())?;
        onkeydown.forget();
    }
    {
        let app = app.clone();
        let onkeyup = Closure::<dyn FnMut(_)>::new(move |e: KeyboardEvent| {
            let mut inp = app.input.borrow_mut();
            match e.key().as_str() {
                "w" | "W" => inp.up = false,
                "s" | "S" => inp.down = false,
                "a" | "A" => inp.left = false,
                "d" | "D" => inp.right = false,
                _ => {}
            }
        });
        document.add_event_listener_with_callback("keyup", onkeyup.as_ref().unchecked_ref())?;
        onkeyup.forget();
    }

    // Mouse aim + shoot
    {
        let app = app.clone();
        let onmove = Closure::<dyn FnMut(_)>::new(move |e: MouseEvent| {
            // Aim vector from center of screen.
            let rect = app.canvas.get_bounding_client_rect();
            let cx = rect.left() + rect.width() * 0.5;
            let cy = rect.top() + rect.height() * 0.5;
            let dx = (e.client_x() as f64) - cx;
            let dy = (e.client_y() as f64) - cy;
            let scale = 1024.0;
            let mut inp = app.input.borrow_mut();
            inp.aim_x = (dx.max(-32767.0).min(32767.0) / rect.width().max(1.0) * scale) as i16;
            inp.aim_y = (dy.max(-32767.0).min(32767.0) / rect.height().max(1.0) * scale) as i16;
        });
        window.add_event_listener_with_callback("mousemove", onmove.as_ref().unchecked_ref())?;
        onmove.forget();
    }
    {
        let app = app.clone();
        let ondown = Closure::<dyn FnMut(_)>::new(move |_e: MouseEvent| {
            app.input.borrow_mut().shoot = true;
        });
        window.add_event_listener_with_callback("mousedown", ondown.as_ref().unchecked_ref())?;
        ondown.forget();
    }
    {
        let app = app.clone();
        let onup = Closure::<dyn FnMut(_)>::new(move |_e: MouseEvent| {
            app.input.borrow_mut().shoot = false;
        });
        window.add_event_listener_with_callback("mouseup", onup.as_ref().unchecked_ref())?;
        onup.forget();
    }

    Ok(())
}

fn send_input(app: &App) {
    if app.ws.ready_state() != WebSocket::OPEN {
        return;
    }

    let mut seq = app.seq.borrow_mut();
    *seq = seq.wrapping_add(1);
    let inp = *app.input.borrow();

    let msg = C2s::Input(Input {
        seq: *seq,
        up: inp.up,
        down: inp.down,
        left: inp.left,
        right: inp.right,
        aim_x: inp.aim_x,
        aim_y: inp.aim_y,
        shoot: inp.shoot,
    });

    let bytes = encode_c2s(&msg);
    let _ = app.ws.send_with_u8_array(&bytes);
}

fn draw(app: &App) {
    let w = app.canvas.width() as f64;
    let h = app.canvas.height() as f64;

    app.ctx.set_fill_style(&"#0b0f14".into());
    app.ctx.fill_rect(0.0, 0.0, w, h);

    // World origin at screen center.
    let ox = w * 0.5;
    let oy = h * 0.5;

    // Draw arena bounds (just a box)
    app.ctx.set_stroke_style(&"#1f2a36".into());
    app.ctx.stroke_rect(ox - 500.0, oy - 300.0, 1000.0, 600.0);

    // Draw players from last snapshot.
    if let Some(snap) = app.last_snapshot.borrow().as_ref() {
        for p in &snap.players {
            let x = ox + (p.x_mm as f64) / 10.0;
            let y = oy + (p.y_mm as f64) / 10.0;

            let is_me = app.me.borrow().map(|me| me == p.id).unwrap_or(false);
            app.ctx
                .set_fill_style(&(if is_me { "#7ee787" } else { "#79c0ff" }).into());
            app.ctx.fill_rect(x - 8.0, y - 8.0, 16.0, 16.0);

            // HP bar
            app.ctx.set_fill_style(&"#e5534b".into());
            let hpw = (p.hp as f64 / 100.0).max(0.0).min(1.0) * 20.0;
            app.ctx.fill_rect(x - 10.0, y - 16.0, hpw, 3.0);
        }
    }
}
