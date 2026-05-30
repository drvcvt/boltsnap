use std::fs;
use std::path::PathBuf;

use eframe::egui;
use image::{DynamicImage, Rgba, RgbaImage, imageops};

use crate::clipboard::copy_to_clipboard;
use crate::paths::temp_png;
use crate::select::prep_compositor_for;
use crate::{Backend, DynResult};

const SIDEBAR_W: f32 = 68.0;
const STATUSBAR_H: f32 = 32.0;
const ICON_BTN: f32 = 44.0;
const ICON_RADIUS: f32 = 7.0;

const MONO_BG: egui::Color32 = egui::Color32::from_gray(16);
const MONO_PANEL: egui::Color32 = egui::Color32::from_gray(24);
const MONO_INK: egui::Color32 = egui::Color32::from_gray(10);
const MONO_HOVER: egui::Color32 = egui::Color32::from_gray(38);
const MONO_SELECTED: egui::Color32 = egui::Color32::from_gray(62);
const MONO_BORDER: egui::Color32 = egui::Color32::from_gray(34);
const MONO_BORDER_HI: egui::Color32 = egui::Color32::from_gray(120);
const MONO_TEXT: egui::Color32 = egui::Color32::from_gray(190);
const MONO_TEXT_HI: egui::Color32 = egui::Color32::from_gray(245);
const MONO_TEXT_DIM: egui::Color32 = egui::Color32::from_gray(120);
const MONO_TEXT_DARK: egui::Color32 = egui::Color32::from_gray(14);
const MONO_ACCENT: egui::Color32 = egui::Color32::from_gray(225);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Tool {
    Move,
    Arrow,
    Pen,
    Rect,
    Highlight,
    Redact,
    Blur,
}

impl Tool {
    fn label(self) -> &'static str {
        match self {
            Self::Move => "Move",
            Self::Arrow => "Arrow",
            Self::Pen => "Pen",
            Self::Rect => "Box",
            Self::Highlight => "Highlight",
            Self::Redact => "Redact",
            Self::Blur => "Blur",
        }
    }

    fn shortcut(self) -> &'static str {
        match self {
            Self::Move => "M",
            Self::Arrow => "A",
            Self::Pen => "P",
            Self::Rect => "R",
            Self::Highlight => "H",
            Self::Redact => "X",
            Self::Blur => "B",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ActionIcon {
    Undo,
    Clear,
    Save,
}

impl ActionIcon {
    fn tooltip(self) -> &'static str {
        match self {
            Self::Undo => "Undo (Ctrl+Z)",
            Self::Clear => "Clear all annotations",
            Self::Save => "Save & copy (Space)",
        }
    }
}

fn thin_separator(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ICON_BTN, 1.0), egui::Sense::hover());
    ui.painter().line_segment(
        [
            egui::pos2(rect.left() + 8.0, rect.center().y),
            egui::pos2(rect.right() - 8.0, rect.center().y),
        ],
        egui::Stroke::new(1.0, MONO_BORDER),
    );
}

fn tool_glyph(tool: Tool) -> &'static str {
    use egui_phosphor::regular as ph;
    match tool {
        Tool::Move => ph::ARROWS_OUT_CARDINAL,
        Tool::Arrow => ph::ARROW_UP_RIGHT,
        Tool::Pen => ph::PENCIL_SIMPLE,
        Tool::Rect => ph::RECTANGLE,
        Tool::Highlight => ph::HIGHLIGHTER,
        Tool::Redact => ph::EYE_SLASH,
        Tool::Blur => ph::DROP_HALF,
    }
}

fn action_glyph(action: ActionIcon) -> &'static str {
    use egui_phosphor::regular as ph;
    match action {
        ActionIcon::Undo => ph::ARROW_COUNTER_CLOCKWISE,
        ActionIcon::Clear => ph::TRASH,
        ActionIcon::Save => ph::FLOPPY_DISK,
    }
}

#[derive(Clone, Debug)]
struct Annotation {
    tool: Tool,
    points: Vec<[f32; 2]>,
}

struct EditorApp {
    output_path: PathBuf,
    copy_after: bool,
    backend: Backend,
    base: RgbaImage,
    texture: egui::TextureHandle,
    annotations: Vec<Annotation>,
    current: Vec<[f32; 2]>,
    tool: Tool,
    status: String,
    finished: bool,
    zoom: f32,
    pan: egui::Vec2,
    show_help: bool,
    saving: Option<std::thread::JoinHandle<Result<(), String>>>,
}

impl EditorApp {
    fn new(
        cc: &eframe::CreationContext<'_>,
        image_path: PathBuf,
        output_path: PathBuf,
        copy_after: bool,
        backend: Backend,
    ) -> DynResult<Self> {
        let mut fonts = egui::FontDefinitions::default();
        egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
        cc.egui_ctx.set_fonts(fonts);

        let mut visuals = egui::Visuals::dark();
        visuals.window_fill = MONO_BG;
        visuals.panel_fill = MONO_BG;
        visuals.extreme_bg_color = MONO_INK;
        visuals.faint_bg_color = MONO_PANEL;
        visuals.widgets.noninteractive.bg_fill = MONO_PANEL;
        visuals.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0, MONO_BORDER);
        visuals.widgets.noninteractive.fg_stroke = egui::Stroke::new(1.0, MONO_TEXT);
        visuals.widgets.inactive.bg_fill = MONO_PANEL;
        visuals.widgets.inactive.weak_bg_fill = MONO_PANEL;
        visuals.widgets.inactive.bg_stroke = egui::Stroke::new(1.0, MONO_BORDER);
        visuals.widgets.inactive.fg_stroke = egui::Stroke::new(1.0, MONO_TEXT);
        visuals.widgets.hovered.bg_fill = MONO_HOVER;
        visuals.widgets.hovered.weak_bg_fill = MONO_HOVER;
        visuals.widgets.hovered.bg_stroke = egui::Stroke::new(1.0, MONO_BORDER_HI);
        visuals.widgets.hovered.fg_stroke = egui::Stroke::new(1.0, MONO_TEXT_HI);
        visuals.widgets.active.bg_fill = MONO_HOVER;
        visuals.widgets.active.weak_bg_fill = MONO_HOVER;
        visuals.widgets.active.bg_stroke = egui::Stroke::new(1.0, MONO_BORDER_HI);
        visuals.widgets.active.fg_stroke = egui::Stroke::new(1.0, MONO_TEXT_HI);
        visuals.selection.bg_fill = MONO_HOVER;
        visuals.selection.stroke = egui::Stroke::new(1.0, MONO_TEXT_HI);
        // Hide egui's resize-grip glyph; it shows in the canvas corner
        // when CSD is off.
        visuals.resize_corner_size = 0.0;
        cc.egui_ctx.set_visuals(visuals);

        let mut style = (*cc.egui_ctx.global_style()).clone();
        style.spacing.item_spacing = egui::vec2(8.0, 6.0);
        style.spacing.button_padding = egui::vec2(10.0, 6.0);
        let r: egui::CornerRadius = 4.into();
        style.visuals.widgets.active.corner_radius = r;
        style.visuals.widgets.hovered.corner_radius = r;
        style.visuals.widgets.inactive.corner_radius = r;
        style.visuals.widgets.noninteractive.corner_radius = r;
        cc.egui_ctx.set_global_style(style);

        let base = image::open(&image_path)?.to_rgba8();
        let size = [base.width() as usize, base.height() as usize];
        let color_image = egui::ColorImage::from_rgba_unmultiplied(size, base.as_raw());
        let texture =
            cc.egui_ctx
                .load_texture("screenshot", color_image, egui::TextureOptions::LINEAR);
        Ok(Self {
            output_path,
            copy_after,
            backend,
            base,
            texture,
            annotations: Vec::new(),
            current: Vec::new(),
            tool: Tool::Arrow,
            status: String::new(),
            finished: false,
            zoom: 1.0,
            pan: egui::Vec2::ZERO,
            show_help: false,
            saving: None,
        })
    }

    fn save_and_maybe_copy(&mut self) {
        if self.saving.is_some() {
            return;
        }
        let base = self.base.clone();
        let annotations = self.annotations.clone();
        let output_path = self.output_path.clone();
        let copy_after = self.copy_after;
        let backend = self.backend;
        self.status = "Saving…".to_string();
        self.saving = Some(std::thread::spawn(move || {
            let rendered = render_annotations(&base, &annotations);
            if let Some(parent) = output_path.parent() {
                fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            // Save as RGB; all annotation passes write alpha=255.
            DynamicImage::ImageRgba8(rendered)
                .to_rgb8()
                .save(&output_path)
                .map_err(|e| e.to_string())?;
            if copy_after {
                copy_to_clipboard(&output_path, backend).map_err(|e| e.to_string())?;
            }
            Ok(())
        }));
    }

    fn poll_saving(&mut self, ctx: &egui::Context) {
        let Some(handle) = self.saving.take() else { return };
        if handle.is_finished() {
            match handle.join() {
                Ok(Ok(())) => {
                    self.status = if self.copy_after {
                        format!("saved + copied {}", self.output_path.display())
                    } else {
                        format!("saved {}", self.output_path.display())
                    };
                    self.finished = true;
                }
                Ok(Err(err)) => self.status = format!("save failed: {err}"),
                Err(_) => self.status = "save failed: worker panicked".to_string(),
            }
        } else {
            self.saving = Some(handle);
            ctx.request_repaint_after(std::time::Duration::from_millis(40));
        }
    }

    fn image_from_pointer(&self, pos: egui::Pos2, rect: egui::Rect) -> [f32; 2] {
        let x = ((pos.x - rect.left()) / rect.width()).clamp(0.0, 1.0) * self.base.width() as f32;
        let y = ((pos.y - rect.top()) / rect.height()).clamp(0.0, 1.0) * self.base.height() as f32;
        [x, y]
    }

    fn screen_point(&self, point: [f32; 2], rect: egui::Rect) -> egui::Pos2 {
        egui::pos2(
            rect.left() + point[0] / self.base.width() as f32 * rect.width(),
            rect.top() + point[1] / self.base.height() as f32 * rect.height(),
        )
    }

    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        let none = egui::Modifiers::NONE;
        let cmd = egui::Modifiers::COMMAND;

        if ctx.input_mut(|i| i.consume_key(none, egui::Key::M)) {
            self.tool = Tool::Move;
        }
        if ctx.input_mut(|i| i.consume_key(none, egui::Key::A)) {
            self.tool = Tool::Arrow;
        }
        if ctx.input_mut(|i| i.consume_key(none, egui::Key::P)) {
            self.tool = Tool::Pen;
        }
        if ctx.input_mut(|i| i.consume_key(none, egui::Key::R)) {
            self.tool = Tool::Rect;
        }
        if ctx.input_mut(|i| i.consume_key(none, egui::Key::H)) {
            self.tool = Tool::Highlight;
        }
        if ctx.input_mut(|i| i.consume_key(none, egui::Key::X)) {
            self.tool = Tool::Redact;
        }
        if ctx.input_mut(|i| i.consume_key(none, egui::Key::B)) {
            self.tool = Tool::Blur;
        }
        if ctx.input_mut(|i| i.consume_key(cmd, egui::Key::Z)) {
            self.annotations.pop();
        }
        if ctx.input_mut(|i| i.consume_key(cmd, egui::Key::Num0)) {
            self.zoom = 1.0;
            self.pan = egui::Vec2::ZERO;
        }
        if ctx.input_mut(|i| i.consume_key(cmd, egui::Key::Plus)) {
            self.zoom = (self.zoom * 1.15).min(6.0);
        }
        if ctx.input_mut(|i| i.consume_key(cmd, egui::Key::Minus)) {
            self.zoom = (self.zoom / 1.15).max(0.25);
        }
        if ctx.input_mut(|i| i.consume_key(none, egui::Key::F1)) {
            self.show_help = !self.show_help;
        }
        if ctx.input_mut(|i| i.consume_key(none, egui::Key::Escape)) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        if ctx.input_mut(|i| i.consume_key(none, egui::Key::Enter))
            || ctx.input_mut(|i| i.consume_key(none, egui::Key::Space))
        {
            self.save_and_maybe_copy();
        }
        let scroll = ctx.input(|i| i.smooth_scroll_delta.y);
        if scroll.abs() > 0.5 {
            let factor = if scroll > 0.0 { 1.06 } else { 1.0 / 1.06 };
            self.zoom = (self.zoom * factor).clamp(0.1, 12.0);
        }
    }

    fn tool_icon_button(&mut self, ui: &mut egui::Ui, tool: Tool) {
        let selected = self.tool == tool;
        let (rect, response) =
            ui.allocate_exact_size(egui::vec2(ICON_BTN, ICON_BTN), egui::Sense::click());
        let painter = ui.painter_at(rect);

        let (fill, fg, border) = if selected {
            (MONO_SELECTED, MONO_TEXT_HI, MONO_BORDER_HI)
        } else if response.hovered() {
            (MONO_HOVER, MONO_TEXT_HI, MONO_BORDER)
        } else {
            (MONO_PANEL, MONO_TEXT, MONO_PANEL)
        };
        painter.rect(
            rect.shrink(0.5),
            ICON_RADIUS,
            fill,
            egui::Stroke::new(1.0, border),
            egui::StrokeKind::Inside,
        );
        if selected {
            let strip = egui::Rect::from_min_max(
                egui::pos2(rect.left() + 3.0, rect.top() + 11.0),
                egui::pos2(rect.left() + 5.0, rect.bottom() - 11.0),
            );
            painter.rect_filled(strip, 1.5, MONO_ACCENT);
        }
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            tool_glyph(tool),
            egui::FontId::proportional(20.0),
            fg,
        );
        painter.text(
            rect.right_bottom() + egui::vec2(-4.0, -2.0),
            egui::Align2::RIGHT_BOTTOM,
            tool.shortcut(),
            egui::FontId::monospace(8.5),
            MONO_TEXT_DIM,
        );

        let _ = response
            .clone()
            .on_hover_text(format!("{} ({})", tool.label(), tool.shortcut()));
        if response.clicked() {
            self.tool = tool;
        }
    }

    fn action_icon_button(
        &mut self,
        ui: &mut egui::Ui,
        action: ActionIcon,
        primary: bool,
    ) -> egui::Response {
        let (rect, response) =
            ui.allocate_exact_size(egui::vec2(ICON_BTN, ICON_BTN), egui::Sense::click());
        let hovered = response.hovered();
        let painter = ui.painter_at(rect);

        let (fill, fg, border) = if primary && hovered {
            (MONO_TEXT_HI, MONO_TEXT_DARK, MONO_TEXT_HI)
        } else if primary {
            (MONO_ACCENT, MONO_TEXT_DARK, MONO_ACCENT)
        } else if hovered {
            (MONO_HOVER, MONO_TEXT_HI, MONO_BORDER)
        } else {
            (MONO_PANEL, MONO_TEXT, MONO_PANEL)
        };
        painter.rect(
            rect.shrink(0.5),
            ICON_RADIUS,
            fill,
            egui::Stroke::new(1.0, border),
            egui::StrokeKind::Inside,
        );
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            action_glyph(action),
            egui::FontId::proportional(20.0),
            fg,
        );
        response.on_hover_text(action.tooltip())
    }

    fn toolbar_ui(&mut self, ui: &mut egui::Ui) {
        ui.spacing_mut().item_spacing = egui::vec2(0.0, 5.0);
        ui.vertical_centered(|ui| {
            ui.add_space(2.0);
            for tool in [
                Tool::Move,
                Tool::Arrow,
                Tool::Pen,
                Tool::Rect,
                Tool::Highlight,
                Tool::Redact,
                Tool::Blur,
            ] {
                self.tool_icon_button(ui, tool);
            }

            ui.add_space(8.0);
            thin_separator(ui);
            ui.add_space(8.0);

            if self
                .action_icon_button(ui, ActionIcon::Undo, false)
                .clicked()
            {
                self.annotations.pop();
            }
            if self
                .action_icon_button(ui, ActionIcon::Clear, false)
                .clicked()
            {
                self.annotations.clear();
            }

            ui.add_space(8.0);
            thin_separator(ui);
            ui.add_space(8.0);

            if self.action_icon_button(ui, ActionIcon::Save, true).clicked() {
                self.save_and_maybe_copy();
            }
        });
    }

    fn statusbar_ui(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(8.0, 0.0);

            ui.label(
                egui::RichText::new(self.tool.label())
                    .color(MONO_TEXT_HI)
                    .strong(),
            );
            ui.label(
                egui::RichText::new(format!("[{}]", self.tool.shortcut()))
                    .color(MONO_TEXT_DIM)
                    .monospace()
                    .small(),
            );
            ui.label(egui::RichText::new("·").color(MONO_TEXT_DIM));
            ui.label(
                egui::RichText::new(format!("{}%", (self.zoom * 100.0).round() as i32))
                    .color(MONO_TEXT)
                    .monospace()
                    .small(),
            );

            ui.with_layout(
                egui::Layout::right_to_left(egui::Align::Center),
                |ui| {
                    let hint = if self.saving.is_some() {
                        ("saving…", MONO_TEXT_HI)
                    } else if self.status.starts_with("save failed") {
                        (
                            self.status.as_str(),
                            egui::Color32::from_rgb(220, 110, 110),
                        )
                    } else if !self.status.is_empty() {
                        (self.status.as_str(), MONO_TEXT)
                    } else {
                        ("space save & copy · esc close", MONO_TEXT_DIM)
                    };
                    ui.label(egui::RichText::new(hint.0).color(hint.1).small());
                },
            );
        });
    }

    fn paint_annotation(
        &self,
        painter: &egui::Painter,
        rect: egui::Rect,
        ann: &Annotation,
        preview: bool,
    ) {
        if ann.points.len() < 2 {
            return;
        }
        let stroke = match ann.tool {
            Tool::Highlight => egui::Stroke::new(3.0, egui::Color32::from_rgb(255, 230, 40)),
            Tool::Redact => egui::Stroke::new(3.0, egui::Color32::BLACK),
            Tool::Blur => egui::Stroke::new(3.0, egui::Color32::from_rgb(115, 190, 255)),
            _ => egui::Stroke::new(
                4.0,
                if preview {
                    egui::Color32::LIGHT_RED
                } else {
                    egui::Color32::from_rgb(255, 70, 70)
                },
            ),
        };
        match ann.tool {
            Tool::Move => {}
            Tool::Pen => {
                for pair in ann.points.windows(2) {
                    painter.line_segment(
                        [
                            self.screen_point(pair[0], rect),
                            self.screen_point(pair[1], rect),
                        ],
                        stroke,
                    );
                }
            }
            Tool::Arrow => {
                let a = self.screen_point(ann.points[0], rect);
                let b = self.screen_point(*ann.points.last().unwrap(), rect);
                painter.line_segment([a, b], stroke);
                let dir = (a - b).normalized();
                let left = egui::vec2(dir.x * 22.0 - dir.y * 10.0, dir.y * 22.0 + dir.x * 10.0);
                let right = egui::vec2(dir.x * 22.0 + dir.y * 10.0, dir.y * 22.0 - dir.x * 10.0);
                painter.line_segment([b, b + left], stroke);
                painter.line_segment([b, b + right], stroke);
            }
            Tool::Rect | Tool::Highlight | Tool::Redact | Tool::Blur => {
                let a = self.screen_point(ann.points[0], rect);
                let b = self.screen_point(ann.points[1], rect);
                let r = egui::Rect::from_two_pos(a, b);
                match ann.tool {
                    Tool::Redact => {
                        painter.rect_filled(r, 3.0, egui::Color32::BLACK);
                    }
                    Tool::Highlight => {
                        painter.rect_filled(
                            r,
                            3.0,
                            egui::Color32::from_rgba_unmultiplied(255, 230, 0, 72),
                        );
                        painter.rect_stroke(r, 3.0, stroke, egui::StrokeKind::Outside);
                    }
                    Tool::Blur => {
                        painter.rect_filled(
                            r,
                            3.0,
                            egui::Color32::from_rgba_unmultiplied(80, 160, 255, 44),
                        );
                        painter.rect_stroke(r, 3.0, stroke, egui::StrokeKind::Outside);
                    }
                    Tool::Rect => {
                        painter.rect_stroke(r, 3.0, stroke, egui::StrokeKind::Outside);
                    }
                    _ => {}
                }
            }
        }
    }

    fn draw_canvas(&mut self, ui: &mut egui::Ui) {
        let img_native = egui::vec2(self.base.width() as f32, self.base.height() as f32);
        let avail = ui.available_size().max(egui::vec2(200.0, 200.0));
        let fit_scale = (avail.x / img_native.x).min(avail.y / img_native.y);
        let scale = (fit_scale * self.zoom).clamp(0.05, 12.0);
        let img_size = img_native * scale;

        let (response, painter) = ui.allocate_painter(avail, egui::Sense::click_and_drag());
        let outer = response.rect;
        painter.rect_filled(outer, 0.0, MONO_INK);

        // Middle-mouse pan in any tool. egui's drag sense only reports
        // primary-button drags so we read raw pointer delta.
        let ctx = ui.ctx().clone();
        let pointer_in = ctx
            .input(|i| i.pointer.hover_pos().map(|p| outer.contains(p)))
            .unwrap_or(false);
        if pointer_in
            && ctx.input(|i| i.pointer.button_down(egui::PointerButton::Middle))
        {
            let delta = ctx.input(|i| i.pointer.delta());
            if delta != egui::Vec2::ZERO {
                self.pan += delta;
            }
        }

        let img_rect =
            egui::Rect::from_center_size(outer.center() + self.pan, img_size);
        let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
        painter.image(self.texture.id(), img_rect, uv, egui::Color32::WHITE);

        // Annotations are primary-button only; egui's drag events fire
        // for any button, so we filter explicitly.
        let primary = egui::PointerButton::Primary;
        if self.tool == Tool::Move {
            if response.dragged_by(primary) {
                self.pan += response.drag_delta();
            }
            if response.double_clicked() {
                self.pan = egui::Vec2::ZERO;
                self.zoom = 1.0;
            }
        }
        if self.tool != Tool::Move {
            if response.drag_started_by(primary)
                && let Some(pos) = response.interact_pointer_pos()
            {
                self.current = vec![self.image_from_pointer(pos, img_rect)];
            }
            if response.dragged_by(primary)
                && let Some(pos) = response.interact_pointer_pos()
            {
                let p = self.image_from_pointer(pos, img_rect);
                if self.tool == Tool::Pen || self.current.len() < 2 {
                    self.current.push(p);
                } else {
                    self.current[1] = p;
                }
            }
            if response.drag_stopped_by(primary) && self.current.len() >= 2 {
                self.annotations.push(Annotation {
                    tool: self.tool,
                    points: self.current.clone(),
                });
                self.current.clear();
            }
        }

        let clip = ui.painter_at(img_rect);
        for ann in &self.annotations {
            self.paint_annotation(&clip, img_rect, ann, false);
        }
        if self.current.len() >= 2 {
            let preview = Annotation {
                tool: self.tool,
                points: self.current.clone(),
            };
            self.paint_annotation(&clip, img_rect, &preview, true);
        }
    }
}

impl eframe::App for EditorApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.handle_shortcuts(&ctx);
        self.poll_saving(&ctx);

        if ctx.input(|i| i.viewport().close_requested()) {
            self.finished = true;
        }
        if self.finished {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }

        let toolbar_frame = egui::Frame::new()
            .fill(MONO_PANEL)
            .stroke(egui::Stroke::NONE)
            .inner_margin(egui::Margin {
                left: 6,
                right: 6,
                top: 12,
                bottom: 12,
            });
        egui::Panel::left("boltsnap-tools")
            .exact_size(SIDEBAR_W)
            .resizable(false)
            .show_separator_line(false)
            .frame(toolbar_frame)
            .show_inside(ui, |ui| {
                self.toolbar_ui(ui);
            });

        let status_frame = egui::Frame::new()
            .fill(MONO_PANEL)
            .stroke(egui::Stroke::NONE)
            .inner_margin(egui::Margin::symmetric(14, 7));
        egui::Panel::bottom("boltsnap-status")
            .exact_size(STATUSBAR_H)
            .resizable(false)
            .show_separator_line(false)
            .frame(status_frame)
            .show_inside(ui, |ui| {
                self.statusbar_ui(ui);
            });

        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(MONO_INK).inner_margin(0))
            .show_inside(ui, |ui| {
                self.draw_canvas(ui);
            });
    }
}

pub fn run_editor(
    image_path: PathBuf,
    output_path: Option<PathBuf>,
    copy_after: bool,
    backend: Backend,
) -> DynResult<PathBuf> {
    let output = output_path.unwrap_or_else(|| temp_png("edited"));
    let out_clone = output.clone();

    let (img_w, img_h) = image::image_dimensions(&image_path)?;
    let max_w = 1700.0_f32;
    let max_h = 980.0_f32;
    let canvas_max_w = max_w - SIDEBAR_W;
    let canvas_max_h = max_h - STATUSBAR_H;
    let scale = (canvas_max_w / img_w as f32)
        .min(canvas_max_h / img_h as f32)
        .min(1.0);
    // Toolbar needs ~560 px or the Save button clips off the bottom.
    const MIN_TOOLBAR_H: f32 = 560.0;
    let win_w = (img_w as f32 * scale + SIDEBAR_W).max(420.0);
    let win_h = (img_h as f32 * scale + STATUSBAR_H).max(MIN_TOOLBAR_H);

    let native = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Boltsnap")
            .with_app_id("boltsnap-editor")
            .with_window_type(egui::X11WindowType::Dialog)
            .with_inner_size([win_w, win_h])
            .with_min_inner_size([420.0, MIN_TOOLBAR_H])
            .with_resizable(false)
            .with_decorations(false)
            .with_always_on_top(),
        ..Default::default()
    };

    prep_compositor_for("boltsnap-editor", false);
    eframe::run_native(
        "Boltsnap Editor",
        native,
        Box::new(move |cc| {
            Ok(Box::new(
                EditorApp::new(cc, image_path, output.clone(), copy_after, backend)
                    .map_err(|e| e.to_string())?,
            ))
        }),
    )?;
    Ok(out_clone)
}

pub fn render_annotations(base: &RgbaImage, annotations: &[Annotation]) -> RgbaImage {
    let mut out = base.clone();
    for ann in annotations {
        if ann.points.len() < 2 {
            continue;
        }
        match ann.tool {
            Tool::Move => {}
            Tool::Pen => {
                for pair in ann.points.windows(2) {
                    draw_thick_line(&mut out, pair[0], pair[1], Rgba([255, 48, 48, 255]), 5.0);
                }
            }
            Tool::Arrow => {
                let a = ann.points[0];
                let b = *ann.points.last().unwrap();
                draw_thick_line(&mut out, a, b, Rgba([255, 48, 48, 255]), 5.0);
                let angle = (b[1] - a[1]).atan2(b[0] - a[0]);
                let len = 26.0;
                for spread in [2.55_f32, -2.55_f32] {
                    let end = [
                        b[0] + len * (angle + spread).cos(),
                        b[1] + len * (angle + spread).sin(),
                    ];
                    draw_thick_line(&mut out, b, end, Rgba([255, 48, 48, 255]), 5.0);
                }
            }
            Tool::Rect => draw_rect_outline(
                &mut out,
                ann.points[0],
                ann.points[1],
                Rgba([255, 48, 48, 255]),
                5,
            ),
            Tool::Highlight => fill_rect_alpha(
                &mut out,
                ann.points[0],
                ann.points[1],
                Rgba([255, 230, 0, 90]),
            ),
            Tool::Redact => fill_rect(&mut out, ann.points[0], ann.points[1], Rgba([0, 0, 0, 255])),
            Tool::Blur => blur_rect(&mut out, ann.points[0], ann.points[1], 10.0),
        }
    }
    out
}

fn blur_rect(img: &mut RgbaImage, a: [f32; 2], b: [f32; 2], sigma: f32) {
    let (x1, y1, x2, y2) = rect_bounds(img, a, b);
    if x2 <= x1 || y2 <= y1 {
        return;
    }
    let w = (x2 - x1 + 1) as u32;
    let h = (y2 - y1 + 1) as u32;
    let crop = imageops::crop_imm(img, x1 as u32, y1 as u32, w, h).to_image();
    let blurred = imageops::blur(&crop, sigma);
    imageops::replace(img, &blurred, x1 as i64, y1 as i64);
}

fn draw_thick_line(img: &mut RgbaImage, a: [f32; 2], b: [f32; 2], color: Rgba<u8>, radius: f32) {
    let dx = b[0] - a[0];
    let dy = b[1] - a[1];
    let steps = dx.abs().max(dy.abs()).max(1.0) as i32;
    for i in 0..=steps {
        let t = i as f32 / steps as f32;
        draw_disc(img, a[0] + dx * t, a[1] + dy * t, radius, color);
    }
}

fn draw_disc(img: &mut RgbaImage, cx: f32, cy: f32, r: f32, color: Rgba<u8>) {
    let r2 = r * r;
    let min_x = (cx - r).floor() as i32;
    let max_x = (cx + r).ceil() as i32;
    let min_y = (cy - r).floor() as i32;
    let max_y = (cy + r).ceil() as i32;
    for y in min_y..=max_y {
        for x in min_x..=max_x {
            if (x as f32 - cx).powi(2) + (y as f32 - cy).powi(2) <= r2 {
                put_pixel_checked(img, x, y, color);
            }
        }
    }
}

fn draw_rect_outline(img: &mut RgbaImage, a: [f32; 2], b: [f32; 2], color: Rgba<u8>, width: i32) {
    let (x1, y1, x2, y2) = rect_bounds(img, a, b);
    for w in 0..width {
        for x in x1..=x2 {
            put_pixel_checked(img, x, y1 + w, color);
            put_pixel_checked(img, x, y2 - w, color);
        }
        for y in y1..=y2 {
            put_pixel_checked(img, x1 + w, y, color);
            put_pixel_checked(img, x2 - w, y, color);
        }
    }
}

fn fill_rect(img: &mut RgbaImage, a: [f32; 2], b: [f32; 2], color: Rgba<u8>) {
    let (x1, y1, x2, y2) = rect_bounds(img, a, b);
    for y in y1..=y2 {
        for x in x1..=x2 {
            put_pixel_checked(img, x, y, color);
        }
    }
}

fn fill_rect_alpha(img: &mut RgbaImage, a: [f32; 2], b: [f32; 2], color: Rgba<u8>) {
    let (x1, y1, x2, y2) = rect_bounds(img, a, b);
    let alpha = color[3] as f32 / 255.0;
    for y in y1..=y2 {
        for x in x1..=x2 {
            if x < 0 || y < 0 || x >= img.width() as i32 || y >= img.height() as i32 {
                continue;
            }
            let p = img.get_pixel_mut(x as u32, y as u32);
            for c in 0..3 {
                p[c] = ((p[c] as f32 * (1.0 - alpha)) + (color[c] as f32 * alpha)).round() as u8;
            }
            p[3] = 255;
        }
    }
}

fn rect_bounds(img: &RgbaImage, a: [f32; 2], b: [f32; 2]) -> (i32, i32, i32, i32) {
    let max_x = img.width().saturating_sub(1) as i32;
    let max_y = img.height().saturating_sub(1) as i32;
    let x1 = (a[0].min(b[0]).round() as i32).clamp(0, max_x);
    let y1 = (a[1].min(b[1]).round() as i32).clamp(0, max_y);
    let x2 = (a[0].max(b[0]).round() as i32).clamp(0, max_x);
    let y2 = (a[1].max(b[1]).round() as i32).clamp(0, max_y);
    (x1, y1, x2, y2)
}

fn put_pixel_checked(img: &mut RgbaImage, x: i32, y: i32, color: Rgba<u8>) {
    if x >= 0 && y >= 0 && x < img.width() as i32 && y < img.height() as i32 {
        img.put_pixel(x as u32, y as u32, color);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::ImageBuffer;

    #[test]
    fn render_redaction_blacks_region() {
        let base: RgbaImage = ImageBuffer::from_pixel(50, 50, Rgba([255, 255, 255, 255]));
        let ann = Annotation {
            tool: Tool::Redact,
            points: vec![[10.0, 10.0], [20.0, 20.0]],
        };
        let out = render_annotations(&base, &[ann]);
        assert_eq!(*out.get_pixel(15, 15), Rgba([0, 0, 0, 255]));
        assert_eq!(*out.get_pixel(5, 5), Rgba([255, 255, 255, 255]));
    }

    #[test]
    fn render_arrow_draws_end() {
        let base: RgbaImage = ImageBuffer::from_pixel(80, 80, Rgba([255, 255, 255, 255]));
        let ann = Annotation {
            tool: Tool::Arrow,
            points: vec![[5.0, 5.0], [60.0, 60.0]],
        };
        let out = render_annotations(&base, &[ann]);
        assert_ne!(*out.get_pixel(60, 60), Rgba([255, 255, 255, 255]));
    }

    #[test]
    fn render_blur_changes_noisy_region() {
        let mut base: RgbaImage = ImageBuffer::from_pixel(40, 40, Rgba([255, 255, 255, 255]));
        for x in 0..20 {
            for y in 0..40 {
                base.put_pixel(x, y, Rgba([0, 0, 0, 255]));
            }
        }
        let ann = Annotation {
            tool: Tool::Blur,
            points: vec![[8.0, 0.0], [25.0, 39.0]],
        };
        let out = render_annotations(&base, &[ann]);
        assert_ne!(*out.get_pixel(18, 20), *base.get_pixel(18, 20));
        assert_eq!(*out.get_pixel(35, 20), Rgba([255, 255, 255, 255]));
    }
}
