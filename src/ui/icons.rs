//! Lucide icons from the embedded sprite (`assets/icons/ui/lucide.svg`),
//! stroked at any size with the theme's stroke width.

use std::sync::OnceLock;

use tiny_skia::{LineCap, LineJoin, Path, PathBuilder, Stroke, Transform};

use super::theme::STROKE;
use super::{Canvas, Rect, Rgb, paint};

const SPRITE: &str = include_str!("../../assets/icons/ui/lucide.svg");
/// Lucide draws on a 24 unit grid.
const GRID: f32 = 24.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Icon {
    ArrowDownLeft,
    ArrowDownRight,
    ArrowUpLeft,
    ArrowUpRight,
    Check,
    ChevronDown,
    Clock,
    Columns2,
    Film,
    Folder,
    FolderOpen,
    GalleryVerticalEnd,
    Gauge,
    HardDriveDownload,
    Image,
    Info,
    Layers,
    MemoryStick,
    Mic,
    Minus,
    Monitor,
    MousePointer2,
    Plus,
    Power,
    RotateCcw,
    RotateCcwClock,
    Scaling,
    Scan,
    SquareDashed,
    Timer,
    TriangleAlert,
    Video,
    Volume2,
}

impl Icon {
    pub const ALL: [Icon; 33] = [
        Icon::ArrowDownLeft,
        Icon::ArrowDownRight,
        Icon::ArrowUpLeft,
        Icon::ArrowUpRight,
        Icon::Check,
        Icon::ChevronDown,
        Icon::Clock,
        Icon::Columns2,
        Icon::Film,
        Icon::Folder,
        Icon::FolderOpen,
        Icon::GalleryVerticalEnd,
        Icon::Gauge,
        Icon::HardDriveDownload,
        Icon::Image,
        Icon::Info,
        Icon::Layers,
        Icon::MemoryStick,
        Icon::Mic,
        Icon::Minus,
        Icon::Monitor,
        Icon::MousePointer2,
        Icon::Plus,
        Icon::Power,
        Icon::RotateCcw,
        Icon::RotateCcwClock,
        Icon::Scaling,
        Icon::Scan,
        Icon::SquareDashed,
        Icon::Timer,
        Icon::TriangleAlert,
        Icon::Video,
        Icon::Volume2,
    ];

    /// The symbol id in the sprite.
    pub fn id(self) -> &'static str {
        match self {
            Icon::ArrowDownLeft => "arrow-down-left",
            Icon::ArrowDownRight => "arrow-down-right",
            Icon::ArrowUpLeft => "arrow-up-left",
            Icon::ArrowUpRight => "arrow-up-right",
            Icon::Check => "check",
            Icon::ChevronDown => "chevron-down",
            Icon::Clock => "clock",
            Icon::Columns2 => "columns-2",
            Icon::Film => "film",
            Icon::Folder => "folder",
            Icon::FolderOpen => "folder-open",
            Icon::GalleryVerticalEnd => "gallery-vertical-end",
            Icon::Gauge => "gauge",
            Icon::HardDriveDownload => "hard-drive-download",
            Icon::Image => "image",
            Icon::Info => "info",
            Icon::Layers => "layers",
            Icon::MemoryStick => "memory-stick",
            Icon::Mic => "mic",
            Icon::Minus => "minus",
            Icon::Monitor => "monitor",
            Icon::MousePointer2 => "mouse-pointer-2",
            Icon::Plus => "plus",
            Icon::Power => "power",
            Icon::RotateCcw => "rotate-ccw",
            Icon::RotateCcwClock => "rotate-ccw-clock",
            Icon::Scaling => "scaling",
            Icon::Scan => "scan",
            Icon::SquareDashed => "square-dashed",
            Icon::Timer => "timer",
            Icon::TriangleAlert => "triangle-alert",
            Icon::Video => "video",
            Icon::Volume2 => "volume-2",
        }
    }

    /// The outline on the 24 unit grid; `None` if the sprite lacks it.
    pub fn path(self) -> Option<&'static Path> {
        static PATHS: OnceLock<Vec<Option<Path>>> = OnceLock::new();
        let paths = PATHS.get_or_init(|| {
            Icon::ALL
                .iter()
                .map(|icon| symbol(SPRITE, icon.id()).and_then(parse))
                .collect()
        });
        let index = Icon::ALL.iter().position(|icon| *icon == self)?;
        paths[index].as_ref()
    }

    /// Stroke the icon into the square `rect`.
    pub fn draw(self, canvas: &mut Canvas, rect: Rect, color: Rgb, alpha: f32) {
        let Some(path) = self.path() else {
            return;
        };
        let size = rect.w.min(rect.h);
        let k = canvas.scale * size / GRID;
        // Whole device pixels for the box, so strokes stay crisp.
        let transform = Transform::from_row(
            k,
            0.0,
            0.0,
            k,
            (rect.x * canvas.scale).round(),
            (rect.y * canvas.scale).round(),
        );
        let stroke = Stroke {
            width: STROKE * GRID / size,
            line_cap: LineCap::Round,
            line_join: LineJoin::Round,
            ..Stroke::default()
        };
        canvas
            .pixmap
            .stroke_path(path, &paint(color, alpha), &stroke, transform, None);
    }
}

/// The body of `<symbol id="id" …>…</symbol>`.
pub fn symbol<'a>(sprite: &'a str, id: &str) -> Option<&'a str> {
    let open = sprite.find(&format!("<symbol id=\"{id}\""))?;
    let body = open + sprite[open..].find('>')? + 1;
    let close = body + sprite[body..].find("</symbol>")?;
    Some(&sprite[body..close])
}

fn attr(element: &str, name: &str) -> Option<f32> {
    attr_str(element, name)?.trim().parse().ok()
}

fn attr_str<'a>(element: &'a str, name: &str) -> Option<&'a str> {
    let key = format!(" {name}=\"");
    let start = element.find(&key)? + key.len();
    let len = element[start..].find('"')?;
    Some(&element[start..start + len])
}

/// All drawing elements of an SVG fragment as one path: `path`, `circle`,
/// `ellipse`, `rect`, `line`, `polyline`, `polygon`.
pub fn parse(svg: &str) -> Option<Path> {
    let mut pb = PathBuilder::new();
    for element in svg.split('<').skip(1) {
        let tag = element.split_whitespace().next().unwrap_or("");
        let a = |name| attr(element, name);
        match tag {
            "path" => path_data(&mut pb, attr_str(element, "d")?)?,
            "circle" => pb.push_circle(a("cx")?, a("cy")?, a("r")?),
            "ellipse" => {
                let (rx, ry) = (a("rx")?, a("ry")?);
                pb.push_oval(tiny_skia::Rect::from_xywh(
                    a("cx")? - rx,
                    a("cy")? - ry,
                    2.0 * rx,
                    2.0 * ry,
                )?);
            }
            "rect" => {
                let rect = Rect::new(
                    a("x").unwrap_or(0.0),
                    a("y").unwrap_or(0.0),
                    a("width")?,
                    a("height")?,
                );
                super::push_rounded(&mut pb, rect, a("rx").or(a("ry")).unwrap_or(0.0));
            }
            "line" => {
                pb.move_to(a("x1")?, a("y1")?);
                pb.line_to(a("x2")?, a("y2")?);
            }
            "polyline" | "polygon" => {
                let mut lexer = Lexer::new(attr_str(element, "points")?);
                pb.move_to(lexer.number()?, lexer.number()?);
                while let Some(x) = lexer.number() {
                    pb.line_to(x, lexer.number()?);
                }
                if tag == "polygon" {
                    pb.close();
                }
            }
            _ => {}
        }
    }
    pb.finish()
}

struct Lexer<'a> {
    s: &'a [u8],
    i: usize,
}

impl<'a> Lexer<'a> {
    fn new(s: &'a str) -> Self {
        Self {
            s: s.as_bytes(),
            i: 0,
        }
    }

    fn skip(&mut self) {
        while self
            .s
            .get(self.i)
            .is_some_and(|c| c.is_ascii_whitespace() || *c == b',')
        {
            self.i += 1;
        }
    }

    fn command(&mut self) -> Option<u8> {
        self.skip();
        let c = *self.s.get(self.i)?;
        c.is_ascii_alphabetic().then(|| {
            self.i += 1;
            c
        })
    }

    fn at_end(&mut self) -> bool {
        self.skip();
        self.i >= self.s.len()
    }

    fn number(&mut self) -> Option<f32> {
        self.skip();
        let start = self.i;
        let digits = |lexer: &mut Self| {
            while lexer.s.get(lexer.i).is_some_and(u8::is_ascii_digit) {
                lexer.i += 1;
            }
        };
        if matches!(self.s.get(self.i), Some(b'-' | b'+')) {
            self.i += 1;
        }
        digits(self);
        if self.s.get(self.i) == Some(&b'.') {
            self.i += 1;
            digits(self);
        }
        if matches!(self.s.get(self.i), Some(b'e' | b'E')) {
            self.i += 1;
            if matches!(self.s.get(self.i), Some(b'-' | b'+')) {
                self.i += 1;
            }
            digits(self);
        }
        let text = std::str::from_utf8(&self.s[start..self.i]).ok()?;
        let value = text.parse().ok();
        if value.is_none() {
            self.i = start;
        }
        value
    }

    /// Arc flags may be written without separators (`a1 1 0 01 2 2`).
    fn flag(&mut self) -> Option<bool> {
        self.skip();
        let c = *self.s.get(self.i)?;
        self.i += 1;
        match c {
            b'0' => Some(false),
            b'1' => Some(true),
            _ => None,
        }
    }
}

/// SVG path data onto `pb`.
fn path_data(pb: &mut PathBuilder, d: &str) -> Option<()> {
    let mut lexer = Lexer::new(d);
    let (mut x, mut y) = (0.0f32, 0.0f32);
    let (mut start_x, mut start_y) = (0.0f32, 0.0f32);
    // Last control point, for S and T.
    let mut cubic: Option<(f32, f32)> = None;
    let mut quad: Option<(f32, f32)> = None;
    let mut command = lexer.command()?;
    loop {
        let relative = command.is_ascii_lowercase();
        let (ox, oy) = if relative { (x, y) } else { (0.0, 0.0) };
        let point = |lexer: &mut Lexer| Some((ox + lexer.number()?, oy + lexer.number()?));
        let (mut next_cubic, mut next_quad) = (None, None);
        match command.to_ascii_uppercase() {
            b'M' => {
                (x, y) = point(&mut lexer)?;
                (start_x, start_y) = (x, y);
                pb.move_to(x, y);
                // Further pairs after a move are lines.
                command = if relative { b'l' } else { b'L' };
            }
            b'L' => {
                (x, y) = point(&mut lexer)?;
                pb.line_to(x, y);
            }
            b'H' => {
                x = ox + lexer.number()?;
                pb.line_to(x, y);
            }
            b'V' => {
                y = oy + lexer.number()?;
                pb.line_to(x, y);
            }
            b'C' => {
                let (c1, c2, end) = (point(&mut lexer)?, point(&mut lexer)?, point(&mut lexer)?);
                pb.cubic_to(c1.0, c1.1, c2.0, c2.1, end.0, end.1);
                next_cubic = Some(c2);
                (x, y) = end;
            }
            b'S' => {
                let c1 = cubic.map_or((x, y), |(cx, cy)| (2.0 * x - cx, 2.0 * y - cy));
                let (c2, end) = (point(&mut lexer)?, point(&mut lexer)?);
                pb.cubic_to(c1.0, c1.1, c2.0, c2.1, end.0, end.1);
                next_cubic = Some(c2);
                (x, y) = end;
            }
            b'Q' => {
                let (c, end) = (point(&mut lexer)?, point(&mut lexer)?);
                pb.quad_to(c.0, c.1, end.0, end.1);
                next_quad = Some(c);
                (x, y) = end;
            }
            b'T' => {
                let c = quad.map_or((x, y), |(cx, cy)| (2.0 * x - cx, 2.0 * y - cy));
                let end = point(&mut lexer)?;
                pb.quad_to(c.0, c.1, end.0, end.1);
                next_quad = Some(c);
                (x, y) = end;
            }
            b'A' => {
                let (rx, ry, rotation) = (lexer.number()?, lexer.number()?, lexer.number()?);
                let (large, sweep) = (lexer.flag()?, lexer.flag()?);
                let end = point(&mut lexer)?;
                arc(pb, (x, y), (rx, ry), rotation, large, sweep, end);
                (x, y) = end;
            }
            b'Z' => {
                pb.close();
                (x, y) = (start_x, start_y);
            }
            _ => return None,
        }
        (cubic, quad) = (next_cubic, next_quad);
        if lexer.at_end() {
            return Some(());
        }
        if let Some(next) = lexer.command() {
            command = next;
        } else if command.eq_ignore_ascii_case(&b'Z') {
            return None;
        }
    }
}

/// An elliptical arc as cubic pieces of at most 90°, after SVG 1.1
/// appendix F.6.5 (endpoint to centre parametrisation).
pub fn arc(
    pb: &mut PathBuilder,
    from: (f32, f32),
    (rx, ry): (f32, f32),
    rotation: f32,
    large: bool,
    sweep: bool,
    to: (f32, f32),
) {
    let (mut rx, mut ry) = (rx.abs() as f64, ry.abs() as f64);
    let (x1, y1, x2, y2) = (from.0 as f64, from.1 as f64, to.0 as f64, to.1 as f64);
    if (x1, y1) == (x2, y2) {
        return;
    }
    if rx == 0.0 || ry == 0.0 {
        pb.line_to(to.0, to.1);
        return;
    }
    let (sin, cos) = (rotation as f64).to_radians().sin_cos();
    let (dx, dy) = ((x1 - x2) / 2.0, (y1 - y2) / 2.0);
    let (x1p, y1p) = (cos * dx + sin * dy, -sin * dx + cos * dy);
    let lambda = (x1p / rx).powi(2) + (y1p / ry).powi(2);
    if lambda > 1.0 {
        rx *= lambda.sqrt();
        ry *= lambda.sqrt();
    }
    let num = (rx * ry).powi(2) - (rx * y1p).powi(2) - (ry * x1p).powi(2);
    let den = (rx * y1p).powi(2) + (ry * x1p).powi(2);
    let sign = if large == sweep { -1.0 } else { 1.0 };
    let coef = sign * (num / den).max(0.0).sqrt();
    let (cxp, cyp) = (coef * rx * y1p / ry, -coef * ry * x1p / rx);
    let (cx, cy) = (
        cos * cxp - sin * cyp + (x1 + x2) / 2.0,
        sin * cxp + cos * cyp + (y1 + y2) / 2.0,
    );
    let angle = |ux: f64, uy: f64, vx: f64, vy: f64| (ux * vy - uy * vx).atan2(ux * vx + uy * vy);
    let (ux, uy) = ((x1p - cxp) / rx, (y1p - cyp) / ry);
    let (vx, vy) = ((-x1p - cxp) / rx, (-y1p - cyp) / ry);
    let start = angle(1.0, 0.0, ux, uy);
    let mut span = angle(ux, uy, vx, vy);
    if !sweep && span > 0.0 {
        span -= std::f64::consts::TAU;
    } else if sweep && span < 0.0 {
        span += std::f64::consts::TAU;
    }
    let pieces = (span.abs() / std::f64::consts::FRAC_PI_2).ceil().max(1.0);
    let step = span / pieces;
    let handle = 4.0 / 3.0 * (step / 4.0).tan();
    let at = |a: f64| {
        let (s, c) = a.sin_cos();
        (
            cx + rx * c * cos - ry * s * sin,
            cy + rx * c * sin + ry * s * cos,
        )
    };
    let tangent = |a: f64| {
        let (s, c) = a.sin_cos();
        (-rx * s * cos - ry * c * sin, -rx * s * sin + ry * c * cos)
    };
    for i in 0..pieces as usize {
        let a0 = start + i as f64 * step;
        let a1 = a0 + step;
        let (p0, p3) = (at(a0), at(a1));
        let (t0, t1) = (tangent(a0), tangent(a1));
        let c1 = (p0.0 + handle * t0.0, p0.1 + handle * t0.1);
        let c2 = (p3.0 - handle * t1.0, p3.1 - handle * t1.1);
        let end = if i + 1 == pieces as usize {
            (x2, y2)
        } else {
            p3
        };
        pb.cubic_to(
            c1.0 as f32,
            c1.1 as f32,
            c2.0 as f32,
            c2.1 as f32,
            end.0 as f32,
            end.1 as f32,
        );
    }
}
