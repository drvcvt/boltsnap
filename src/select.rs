use std::env;
use std::process::{Command, Stdio};

use eframe::egui;
use image::RgbaImage;

use crate::DynResult;
use crate::paths::has_cmd;

// Push windowrules upfront so the window appears already floating with
// no fade-in. Beats polling after the window opens — no race, no jank,
// no compositor animation hitting the user before we react.
pub fn prep_compositor_for(class: &str, fullscreen: bool) {
    if env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_some() && has_cmd("hyprctl") {
        let selector = format!("class:^({class})$");
        let mut rules: Vec<String> = vec![
            format!("noanim, {selector}"),
            format!("noblur, {selector}"),
            format!("noshadow, {selector}"),
            format!("float, {selector}"),
            format!("center, {selector}"),
            format!("pin, {selector}"),
        ];
        if fullscreen {
            rules.push(format!("fullscreen, {selector}"));
            rules.push(format!("noborder, {selector}"));
            rules.push(format!("rounding 0, {selector}"));
        }
        for rule in &rules {
            let _ = Command::new("hyprctl")
                .args(["keyword", "windowrulev2", rule])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
    }
    if env::var_os("SWAYSOCK").is_some() && has_cmd("swaymsg") {
        let _ = Command::new("swaymsg")
            .args([
                "for_window",
                &format!("[app_id=\"{class}\"]"),
                if fullscreen {
                    "floating enable, fullscreen enable, border none"
                } else {
                    "floating enable"
                },
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

// Single-process selection: spawn the compositor capture on a worker
// thread so it overlaps with eframe's winit + GL init in the main
// thread, then run the SelectApp inline. No child process, no PNG/raw
// handoff over a pipe, no extra Rust cold-start.
//
// Returns the cropped RgbaImage on confirm, or None on Esc/cancel.
pub fn run_select_with_parallel_capture<F>(capture: F) -> DynResult<Option<RgbaImage>>
where
    F: FnOnce() -> Result<RgbaImage, String> + Send + 'static,
{
    let capture_handle = std::thread::spawn(capture);

    let result: std::sync::Arc<std::sync::Mutex<Option<RgbaImage>>> =
        std::sync::Arc::new(std::sync::Mutex::new(None));
    let result_clone = result.clone();

    let native = eframe::NativeOptions {
        renderer: eframe::Renderer::Glow,
        persist_window: false,
        persistence_path: None,
        viewport: egui::ViewportBuilder::default()
            .with_title("boltsnap-select")
            .with_app_id("boltsnap-select")
            .with_decorations(false)
            .with_resizable(false)
            .with_fullscreen(true)
            .with_active(true)
            .with_always_on_top()
            .with_window_type(egui::X11WindowType::Splash),
        ..Default::default()
    };

    // Sync: the hyprctl/swaymsg call is fast (<15 ms) and the rule
    // MUST be in the compositor's table before eframe maps the
    // toplevel, or the default fade-in plays and the overlay feels
    // laggy. Off-threading this loses that race.
    prep_compositor_for("boltsnap-select", true);

    eframe::run_native(
        "boltsnap-select",
        native,
        Box::new(move |cc| {
            // By the time eframe gets here it has spent its winit + GL
            // setup time; any leftover wait against the capture thread
            // is the *parallel* overlap we wanted.
            let image = capture_handle
                .join()
                .map_err(|_| "capture worker panicked".to_string())??;
            Ok(Box::new(SelectApp::new(cc, image, result_clone)))
        }),
    )?;

    Ok(result.lock().unwrap().take())
}

struct SelectApp {
    base_w: u32,
    base_h: u32,
    texture: egui::TextureHandle,
    drag_start: Option<egui::Pos2>,
    drag_now: Option<egui::Pos2>,
    finalized: bool,
    base: RgbaImage,
    result: std::sync::Arc<std::sync::Mutex<Option<RgbaImage>>>,
}

impl SelectApp {
    fn new(
        cc: &eframe::CreationContext<'_>,
        base: RgbaImage,
        result: std::sync::Arc<std::sync::Mutex<Option<RgbaImage>>>,
    ) -> Self {
        let (w, h) = (base.width(), base.height());
        let color =
            egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], base.as_raw());
        let texture =
            cc.egui_ctx
                .load_texture("boltsnap-select-bg", color, egui::TextureOptions::LINEAR);
        Self {
            base_w: w,
            base_h: h,
            texture,
            drag_start: None,
            drag_now: None,
            finalized: false,
            base,
            result,
        }
    }

    fn rect_to_image(&self, p: egui::Pos2, rect: egui::Rect) -> (u32, u32) {
        let nx = ((p.x - rect.left()) / rect.width().max(1.0)).clamp(0.0, 1.0);
        let ny = ((p.y - rect.top()) / rect.height().max(1.0)).clamp(0.0, 1.0);
        (
            (nx * self.base_w as f32).round() as u32,
            (ny * self.base_h as f32).round() as u32,
        )
    }
}

impl eframe::App for SelectApp {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0, 0.0, 0.0, 1.0]
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if self.finalized {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        if ctx.input(|i| i.key_pressed(egui::Key::Escape) || i.viewport().close_requested()) {
            *self.result.lock().unwrap() = None;
            self.finalized = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }

        let frame = egui::Frame::new()
            .fill(egui::Color32::BLACK)
            .inner_margin(0);
        egui::CentralPanel::default()
            .frame(frame)
            .show_inside(ui, |ui| {
                let rect = ui.max_rect();
                let response = ui.allocate_rect(rect, egui::Sense::click_and_drag());
                let painter = ui.painter();

                let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
                painter.image(self.texture.id(), rect, uv, egui::Color32::WHITE);

                if response.drag_started_by(egui::PointerButton::Primary) {
                    if let Some(p) = response.interact_pointer_pos() {
                        self.drag_start = Some(p);
                        self.drag_now = Some(p);
                    }
                }
                if response.dragged_by(egui::PointerButton::Primary) {
                    if let Some(p) = ctx.pointer_latest_pos() {
                        self.drag_now = Some(p);
                    }
                }
                if response.drag_stopped_by(egui::PointerButton::Primary) {
                    if let (Some(a), Some(b)) = (self.drag_start, self.drag_now) {
                        let (ax, ay) = self.rect_to_image(a, rect);
                        let (bx, by) = self.rect_to_image(b, rect);
                        let x = ax.min(bx);
                        let y = ay.min(by);
                        let w = ax.max(bx).saturating_sub(x);
                        let h = ay.max(by).saturating_sub(y);
                        if w > 1 && h > 1 {
                            // Crop here so the parent doesn't have to keep
                            // a copy of the full base image around.
                            let cropped =
                                image::imageops::crop_imm(&self.base, x, y, w, h).to_image();
                            *self.result.lock().unwrap() = Some(cropped);
                            self.finalized = true;
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        } else {
                            self.drag_start = None;
                            self.drag_now = None;
                        }
                    }
                }

                let dim = egui::Color32::from_rgba_unmultiplied(0, 0, 0, 110);
                match (self.drag_start, self.drag_now) {
                    (Some(a), Some(b)) => {
                        let sel = egui::Rect::from_two_pos(a, b);
                        let outside_top =
                            egui::Rect::from_min_max(rect.min, egui::pos2(rect.right(), sel.top()));
                        let outside_bottom = egui::Rect::from_min_max(
                            egui::pos2(rect.left(), sel.bottom()),
                            rect.max,
                        );
                        let outside_left = egui::Rect::from_min_max(
                            egui::pos2(rect.left(), sel.top()),
                            egui::pos2(sel.left(), sel.bottom()),
                        );
                        let outside_right = egui::Rect::from_min_max(
                            egui::pos2(sel.right(), sel.top()),
                            egui::pos2(rect.right(), sel.bottom()),
                        );
                        for r in [outside_top, outside_bottom, outside_left, outside_right] {
                            if r.width() > 0.0 && r.height() > 0.0 {
                                painter.rect_filled(r, 0.0, dim);
                            }
                        }
                        painter.rect_stroke(
                            sel,
                            0.0,
                            egui::Stroke::new(1.5, egui::Color32::WHITE),
                            egui::StrokeKind::Outside,
                        );
                        let label = format!(
                            "{}x{}",
                            sel.width().round() as i32,
                            sel.height().round() as i32
                        );
                        painter.text(
                            sel.left_top() + egui::vec2(6.0, -4.0),
                            egui::Align2::LEFT_BOTTOM,
                            label,
                            egui::FontId::monospace(12.0),
                            egui::Color32::WHITE,
                        );
                    }
                    _ => {
                        painter.rect_filled(rect, 0.0, dim);
                        painter.text(
                            rect.center(),
                            egui::Align2::CENTER_CENTER,
                            "drag to select • Esc to cancel",
                            egui::FontId::proportional(14.0),
                            egui::Color32::from_white_alpha(220),
                        );
                    }
                }
            });
    }
}
