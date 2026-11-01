//! Live popup check with owned surfaces only (R6). Run inside a Wayland session:
//! `cargo run --no-default-features --features window --example popup`.
//!
//! 1. A 480x320 window opens; a click-through tooltip shows under its top edge
//!    for 4 seconds (move the mouse across it; it counts pointer events).
//! 2. Left-click in the window: a grabbing 200x160 menu opens at the click.
//! 3. Click the menu's upper half: a grabbing 160x120 submenu opens to the
//!    right. Press any key once: the log says which surface got it.
//! 4. Press Space: the menu moves 40 px down (reposition).
//! 5. Click outside the window: the compositor dismisses submenu and menu.
//! 6. Left-click again, then Escape twice: submenu, then menu close.
//! 7. Press L: a 240x120 layer opens top right. Click it: a grabbing popup
//!    opens below the click. Press Escape, click the layer again, then click
//!    the layer once more while that popup is open.
//!
//! Static calm frames, no animation or flashing. Every popup closes by itself
//! after 15 seconds without input; the example exits after 120 seconds or
//! when the window is closed.
use libway::window::{
    Anchor, Configuration, ConstraintAdjustment, ExclusiveZone, Host, InputEvent, InputObservation,
    InputRegion, InputSeatId, KeyboardInteractivity, KeyboardObservation, Layer, LayerOptions,
    LayerState, LogicalRect, PointerObservation, PopupAnchor, PopupGravity, PopupOptions,
    WindowEvent, WindowId, WindowOptions,
};
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

const BTN_LEFT: u32 = 0x110;
const KEY_ESCAPE: u32 = 1;
const KEY_SPACE: u32 = 57;
const KEY_L: u32 = 38;
const IDLE: Duration = Duration::from_secs(15);
const TOOLTIP_TIME: Duration = Duration::from_secs(4);
const BORDER: [u8; 4] = [150, 160, 175, 255];

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Main,
    Tooltip,
    Menu,
    Submenu,
    Layer,
    LayerPopup,
}

impl Kind {
    fn name(self) -> &'static str {
        match self {
            Kind::Main => "parent",
            Kind::Tooltip => "tooltip",
            Kind::Menu => "menu",
            Kind::Submenu => "submenu",
            Kind::Layer => "layer",
            Kind::LayerPopup => "layer popup",
        }
    }

    fn fill(self) -> [u8; 4] {
        match self {
            Kind::Main => [26, 29, 35, 255],
            Kind::Tooltip => [58, 54, 36, 255],
            Kind::Menu => [44, 48, 58, 255],
            Kind::Submenu => [52, 58, 70, 255],
            Kind::Layer => [36, 40, 48, 255],
            Kind::LayerPopup => [48, 44, 58, 255],
        }
    }
}

struct Surface {
    kind: Kind,
    configuration: Option<Configuration>,
    dirty: bool,
    pointer: (f64, f64),
}

struct App {
    host: Host,
    surfaces: BTreeMap<WindowId, Surface>,
    main: Option<WindowId>,
    seat: Option<InputSeatId>,
    tooltip_pointer: u32,
    tooltip_shown: bool,
    closing_by_us: Vec<WindowId>,
    key_probe: bool,
    last_input: Instant,
}

fn main() -> libway::Result<()> {
    let mut app = App {
        host: Host::connect()?,
        surfaces: BTreeMap::new(),
        main: None,
        seat: None,
        tooltip_pointer: 0,
        tooltip_shown: false,
        closing_by_us: Vec::new(),
        key_probe: false,
        last_input: Instant::now(),
    };
    let start = Instant::now();
    let deadline = start + Duration::from_secs(120);
    while Instant::now() < deadline {
        app.host
            .pump(Some(Instant::now() + Duration::from_millis(50)))
            .map_err(|failure| failure.error)?;
        for event in app.host.events() {
            if !app.event(event)? {
                app.host.shutdown();
                return Ok(());
            }
        }
        app.present()?;
        app.timers(start)?;
    }
    app.host.shutdown();
    Ok(())
}

impl App {
    fn find(&self, kind: Kind) -> Option<WindowId> {
        self.surfaces
            .iter()
            .find(|(_, s)| s.kind == kind)
            .map(|(id, _)| *id)
    }

    fn name(&self, id: WindowId) -> &'static str {
        self.surfaces.get(&id).map_or("unknown", |s| s.kind.name())
    }

    fn add(&mut self, id: WindowId, kind: Kind) -> libway::Result<()> {
        self.host.enable_cpu(id)?;
        self.surfaces.insert(
            id,
            Surface {
                kind,
                configuration: None,
                dirty: false,
                pointer: (0.0, 0.0),
            },
        );
        Ok(())
    }

    /// Close a popup ourselves; its `Closed` is then not a dismissal.
    fn close(&mut self, id: WindowId, why: &str) -> libway::Result<()> {
        println!("{why} closed {}", self.name(id));
        self.closing_by_us.push(id);
        self.host.close(id)
    }

    fn event(&mut self, event: WindowEvent) -> libway::Result<bool> {
        match event {
            WindowEvent::Ready => {
                let id = self.host.create_window(WindowOptions {
                    preferred_width: 480,
                    preferred_height: 320,
                    title: "libway popup".into(),
                    app_id: "libway.popup".into(),
                })?;
                self.main = Some(id);
                self.add(id, Kind::Main)?;
            }
            WindowEvent::Configured {
                window,
                configuration,
            } => self.configured(window, configuration)?,
            WindowEvent::RedrawRequested { window, .. } => {
                if let Some(surface) = self.surfaces.get_mut(&window) {
                    surface.dirty = true;
                }
            }
            WindowEvent::PopupGrabRefused { window } => {
                println!("{} grab refused", self.name(window));
            }
            WindowEvent::Closed { window } => {
                let ours = self.closing_by_us.contains(&window);
                if let Some(surface) = self.surfaces.remove(&window)
                    && !ours
                {
                    println!("{} closed (popup_done)", surface.kind.name());
                }
            }
            WindowEvent::CloseRequested { window } if Some(window) == self.main => {
                println!("closed by the user");
                return Ok(false);
            }
            WindowEvent::Input(InputEvent { seat, observation }) => {
                self.seat = Some(seat);
                self.input(observation)?;
            }
            _ => {}
        }
        Ok(true)
    }

    fn configured(&mut self, window: WindowId, configuration: Configuration) -> libway::Result<()> {
        let parent_scale = self
            .main
            .and_then(|main| self.surfaces.get(&main))
            .and_then(|s| s.configuration)
            .map_or(0, |c| c.scale_120);
        let Some(surface) = self.surfaces.get_mut(&window) else {
            return Ok(());
        };
        let first = surface.configuration.is_none();
        surface.configuration = Some(configuration);
        surface.dirty = true;
        let kind = surface.kind;
        if let Some(placement) = configuration.placement {
            if let Some(token) = placement.repositioned {
                println!(
                    "{} repositioned token={token} at {},{}",
                    kind.name(),
                    placement.x,
                    placement.y
                );
            } else if first {
                println!(
                    "{} configured {}x{} at {},{} scale_120={} (parent scale_120={parent_scale})",
                    kind.name(),
                    configuration.width,
                    configuration.height,
                    placement.x,
                    placement.y,
                    configuration.scale_120
                );
            }
        }
        Ok(())
    }

    fn input(&mut self, observation: InputObservation) -> libway::Result<()> {
        match observation {
            InputObservation::PointerFrame(frame) => {
                for record in frame.records() {
                    self.pointer(record)?;
                }
            }
            InputObservation::Keyboard(KeyboardObservation::Enter { window, .. }) => {
                println!("keyboard enter on {}", self.name(window));
            }
            InputObservation::Keyboard(KeyboardObservation::Key {
                window,
                physical,
                down: true,
                ..
            }) => {
                self.last_input = Instant::now();
                self.key(window, physical)?;
            }
            _ => {}
        }
        Ok(())
    }

    fn pointer(&mut self, record: &PointerObservation) -> libway::Result<()> {
        match record {
            PointerObservation::Enter { window, x, y, .. }
            | PointerObservation::Motion { window, x, y, .. } => {
                if self
                    .surfaces
                    .get(window)
                    .is_some_and(|s| s.kind == Kind::Tooltip)
                {
                    self.tooltip_pointer += 1;
                }
                if let Some(surface) = self.surfaces.get_mut(window) {
                    surface.pointer = (*x, *y);
                }
            }
            PointerObservation::Button {
                window,
                button: BTN_LEFT,
                down: true,
                ..
            } => {
                self.last_input = Instant::now();
                self.click(*window)?;
            }
            _ => {}
        }
        Ok(())
    }

    fn click(&mut self, window: WindowId) -> libway::Result<()> {
        let Some(surface) = self.surfaces.get(&window) else {
            return Ok(());
        };
        let (x, y) = (surface.pointer.0 as i32, surface.pointer.1 as i32);
        let at = LogicalRect {
            x: x.max(0),
            y: y.max(0),
            width: 1,
            height: 1,
        };
        let all = ConstraintAdjustment::FLIP_X
            | ConstraintAdjustment::FLIP_Y
            | ConstraintAdjustment::SLIDE_X
            | ConstraintAdjustment::SLIDE_Y;
        match surface.kind {
            Kind::Main if self.find(Kind::Menu).is_none() => {
                let menu = self.host.create_popup(PopupOptions {
                    size: (200, 160),
                    anchor_rect: at,
                    anchor: PopupAnchor::TopLeft,
                    gravity: PopupGravity::BottomRight,
                    constraint_adjustment: all,
                    grab: self.seat,
                    ..PopupOptions::new(window)
                })?;
                self.add(menu, Kind::Menu)?;
            }
            Kind::Menu if y < 80 && self.find(Kind::Submenu).is_none() => {
                let submenu = self.host.create_popup(PopupOptions {
                    size: (160, 120),
                    anchor_rect: LogicalRect {
                        x: 0,
                        y: at.y,
                        width: 200,
                        height: 1,
                    },
                    anchor: PopupAnchor::TopRight,
                    gravity: PopupGravity::BottomRight,
                    constraint_adjustment: ConstraintAdjustment::FLIP_X
                        | ConstraintAdjustment::SLIDE_Y,
                    grab: self.seat,
                    ..PopupOptions::new(window)
                })?;
                self.add(submenu, Kind::Submenu)?;
                self.key_probe = true;
            }
            Kind::Layer if self.find(Kind::LayerPopup).is_none() => {
                let popup = self.host.create_popup(PopupOptions {
                    size: (160, 120),
                    anchor_rect: at,
                    anchor: PopupAnchor::BottomLeft,
                    gravity: PopupGravity::BottomRight,
                    constraint_adjustment: all,
                    grab: self.seat,
                    ..PopupOptions::new(window)
                })?;
                self.add(popup, Kind::LayerPopup)?;
            }
            _ => {}
        }
        Ok(())
    }

    fn key(&mut self, window: WindowId, physical: u32) -> libway::Result<()> {
        if self.key_probe {
            self.key_probe = false;
            println!("key on {}", self.name(window));
        }
        match physical {
            KEY_ESCAPE => {
                let topmost = [Kind::Submenu, Kind::Menu, Kind::LayerPopup]
                    .into_iter()
                    .find_map(|kind| self.find(kind));
                if let Some(id) = topmost {
                    self.close(id, "escape")?;
                }
            }
            KEY_SPACE => {
                if let (Some(menu), Some(main)) = (self.find(Kind::Menu), self.main)
                    && let Some(placement) =
                        self.surfaces[&menu].configuration.and_then(|c| c.placement)
                {
                    let rect = LogicalRect {
                        x: placement.x.max(0),
                        y: (placement.y + 40).max(0),
                        width: 1,
                        height: 1,
                    };
                    let parent = self.surfaces[&main].configuration;
                    let fits = parent
                        .is_some_and(|c| (rect.x as u32) < c.width && (rect.y as u32) < c.height);
                    if fits {
                        self.host.reposition_popup(
                            menu,
                            &PopupOptions {
                                size: (200, 160),
                                anchor_rect: rect,
                                anchor: PopupAnchor::TopLeft,
                                gravity: PopupGravity::BottomRight,
                                ..PopupOptions::new(main)
                            },
                        )?;
                    }
                }
            }
            KEY_L if self.find(Kind::Layer).is_none() => {
                let layer = self.host.create_layer(LayerOptions {
                    namespace: "libway.popup-probe".into(),
                    output: None,
                    state: LayerState {
                        layer: Layer::Top,
                        anchor: Anchor::TOP | Anchor::RIGHT,
                        width: 240,
                        height: 120,
                        margin: [24, 24, 0, 0],
                        exclusive_zone: ExclusiveZone::Respect,
                        exclusive_edge: None,
                        keyboard: KeyboardInteractivity::None,
                    },
                })?;
                self.add(layer, Kind::Layer)?;
                println!("layer opened");
            }
            _ => {}
        }
        Ok(())
    }

    fn present(&mut self) -> libway::Result<()> {
        let ids: Vec<WindowId> = self.surfaces.keys().copied().collect();
        for id in ids {
            let surface = &self.surfaces[&id];
            let (Some(config), true) = (surface.configuration, surface.dirty) else {
                continue;
            };
            let pixels = frame(config.pixel_width, config.pixel_height, surface.kind.fill());
            self.host.present_cpu(id, config.revision, &pixels)?;
            let kind = surface.kind;
            self.surfaces.get_mut(&id).expect("surface").dirty = false;
            if kind == Kind::Main && !self.tooltip_shown {
                self.tooltip(id, config)?;
            }
        }
        Ok(())
    }

    fn tooltip(&mut self, main: WindowId, config: Configuration) -> libway::Result<()> {
        let tooltip = self.host.create_popup(PopupOptions {
            size: (160, 32),
            anchor_rect: LogicalRect {
                x: 0,
                y: 0,
                width: config.width,
                height: 1,
            },
            anchor: PopupAnchor::Bottom,
            gravity: PopupGravity::Bottom,
            ..PopupOptions::new(main)
        })?;
        self.host.set_input_region(tooltip, InputRegion::None)?;
        self.add(tooltip, Kind::Tooltip)?;
        self.tooltip_shown = true;
        Ok(())
    }

    fn timers(&mut self, start: Instant) -> libway::Result<()> {
        if start.elapsed() >= TOOLTIP_TIME
            && let Some(tooltip) = self.find(Kind::Tooltip)
        {
            println!("tooltip pointer events: {}", self.tooltip_pointer);
            self.close(tooltip, "timeout")?;
        }
        if self.last_input.elapsed() >= IDLE {
            for kind in [Kind::Submenu, Kind::Menu, Kind::LayerPopup] {
                if let Some(id) = self.find(kind) {
                    self.close(id, "idle")?;
                }
            }
            self.last_input = Instant::now();
        }
        Ok(())
    }
}

/// A flat fill with a 2 pixel border.
fn frame(width: u32, height: u32, fill: [u8; 4]) -> Vec<u8> {
    let (width, height) = (width as usize, height as usize);
    let mut pixels = Vec::with_capacity(width * height * 4);
    for y in 0..height {
        for x in 0..width {
            let edge = x < 2 || y < 2 || x + 2 >= width || y + 2 >= height;
            pixels.extend_from_slice(if edge { &BORDER } else { &fill });
        }
    }
    pixels
}
