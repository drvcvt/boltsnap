//! Persistent selector frame and bounded damage history, independent of Wayland.
use super::render;
use std::collections::VecDeque;
use tiny_skia::Pixmap;

pub type Region = (u32, u32, u32, u32); // exclusive right/bottom

#[derive(Clone, Debug, PartialEq)]
pub struct Scene {
    pub selection: Option<(f32, f32, f32, f32)>,
    pub editing: bool,
    pub record: bool,
    pub toggles: (bool, bool, bool),
    pub hovered: Option<usize>,
    // Ignore pointer motion unless it changes the visible loupe or hover.
    pub magnifier: Option<(f64, f64)>,
}

fn intersect(a: Region, b: Region) -> Option<Region> {
    let r = (a.0.max(b.0), a.1.max(b.1), a.2.min(b.2), a.3.min(b.3));
    (r.0 < r.2 && r.1 < r.3).then_some(r)
}

fn subtract(a: Region, b: Region) -> Vec<Region> {
    let Some(i) = intersect(a, b) else {
        return vec![a];
    };
    [
        (a.0, a.1, a.2, i.1),
        (a.0, i.3, a.2, a.3),
        (a.0, i.1, i.0, i.3),
        (i.2, i.1, a.2, i.3),
    ]
    .into_iter()
    .filter(|r| r.0 < r.2 && r.1 < r.3)
    .collect()
}

fn bounds(r: (f64, f64, f64, f64), pad: f64, w: u32, h: u32) -> Option<Region> {
    let (x, y, rw, rh) = r;
    if ![x, y, rw, rh].iter().all(|n| n.is_finite()) || rw <= 0.0 || rh <= 0.0 {
        return None;
    }
    let r = (
        (x - pad).floor().clamp(0.0, w as f64) as u32,
        (y - pad).floor().clamp(0.0, h as f64) as u32,
        (x + rw + pad).ceil().clamp(0.0, w as f64) as u32,
        (y + rh + pad).ceil().clamp(0.0, h as f64) as u32,
    );
    (r.0 < r.2 && r.1 < r.3).then_some(r)
}

fn selection(s: &Scene, w: u32, h: u32) -> Option<Region> {
    let (x, y, rw, rh) = s.selection?;
    if rw < 1.0 || rh < 1.0 {
        return None;
    }
    bounds((x as f64, y as f64, rw as f64, rh as f64), 0.0, w, h)
}

fn decorations(s: &Scene, w: u32, h: u32) -> Vec<Region> {
    let mut regions = Vec::new();
    let mut add = |r, pad| {
        if let Some(r) = bounds(r, pad, w, h) {
            regions.push(r);
        }
    };
    if let Some(sel @ (x, y, rw, rh)) = s.selection.filter(|r| r.2 >= 1.0 && r.3 >= 1.0) {
        let (x, y, rw, rh) = (x as f64, y as f64, rw as f64, rh as f64);
        // Includes anti-aliasing, outlines and corner/midpoint handles. Four
        // strips avoid damaging the unchanged interior of a large selection.
        let pad = if s.editing { 8.0 } else { 4.0 };
        for r in [
            (x, y, rw, 1.0),
            (x, y + rh, rw, 1.0),
            (x, y, 1.0, rh),
            (x + rw, y, 1.0, rh),
        ] {
            add(r, pad);
        }
        let panel = if s.record {
            render::record_toolbar(sel, w, h).map(|t| t.bounds)
        } else {
            render::badge_bounds(sel, w, h)
        };
        if let Some(r) = panel {
            add(r, 2.0);
        }
    }
    if let Some(cursor) = s.magnifier {
        let (x, y) = super::edit::magnifier_placement(cursor, 120.0, 24.0, w as f64, h as f64);
        add((x.round(), y.round(), 120.0, 120.0), 3.0);
    }
    regions
}

/// Disjoint bounded regions. Never merge crossing border strips into the full
/// selection rectangle. Fall back before pathological fragmentation grows.
fn compact(input: impl IntoIterator<Item = Region>, w: u32, h: u32) -> Vec<Region> {
    let mut out = Vec::new();
    for r in input {
        let mut pieces = vec![r];
        for &existing in &out {
            pieces = pieces
                .into_iter()
                .flat_map(|p| subtract(p, existing))
                .collect();
            if pieces.len() + out.len() > 128 {
                return vec![(0, 0, w, h)];
            }
        }
        out.extend(pieces);
        let area: u64 = out
            .iter()
            .map(|r| (r.2 - r.0) as u64 * (r.3 - r.1) as u64)
            .sum();
        if out.len() > 128 || area * 5 > w as u64 * h as u64 * 3 {
            return vec![(0, 0, w, h)];
        }
    }
    out
}

pub fn paint(frame: &mut Pixmap, base: &Pixmap, scene: &Scene) {
    let (w, h) = (base.width(), base.height());
    if let Some(sel) = scene.selection {
        render::draw_border(frame, sel);
        if scene.editing {
            render::draw_handles(frame, sel);
        }
        if scene.record {
            if let Some(toolbar) = render::record_toolbar(sel, w, h) {
                render::draw_record_toolbar(frame, &toolbar, scene.toggles, scene.hovered);
            }
        } else {
            render::draw_badge(frame, sel, w, h);
        }
    }
    if let Some(cursor) = scene.magnifier {
        render::draw_magnifier(frame, base, cursor, w, h);
    }
}

pub struct SceneCache {
    overlay: render::CachedOverlay,
    previous: Option<Scene>,
    generation: u64,
    history: VecDeque<(u64, Vec<Region>)>,
}

impl SceneCache {
    pub fn new(base: &Pixmap) -> Self {
        Self {
            overlay: render::CachedOverlay::new(base),
            previous: None,
            generation: 0,
            history: VecDeque::new(),
        }
    }
    pub fn update(&mut self, base: &Pixmap, scene: Scene) -> bool {
        if self.previous.as_ref() == Some(&scene) {
            return false;
        }
        let (w, h) = (base.width(), base.height());
        let regions = if let Some(old) = &self.previous {
            let mut regions = decorations(old, w, h);
            regions.extend(decorations(&scene, w, h));
            match (selection(old, w, h), selection(&scene, w, h)) {
                (Some(a), Some(b)) => {
                    regions.extend(subtract(a, b));
                    regions.extend(subtract(b, a));
                }
                (Some(a), None) | (None, Some(a)) => regions.push(a),
                _ => {}
            }
            compact(regions, w, h)
        } else {
            vec![(0, 0, w, h)]
        };
        let frame = self.overlay.reset_regions(base, scene.selection, &regions);
        paint(frame, base, &scene);
        self.generation += 1;
        self.history.push_back((self.generation, regions));
        if self.history.len() > 8 {
            self.history.pop_front();
        }
        self.previous = Some(scene);
        true
    }
    pub fn generation(&self) -> u64 {
        self.generation
    }
    pub fn frame(&self) -> &Pixmap {
        self.overlay.frame()
    }
    pub fn damage_since(&self, generation: Option<u64>) -> Vec<Region> {
        let (w, h) = (self.frame().width(), self.frame().height());
        let Some(generation) = generation else {
            return vec![(0, 0, w, h)];
        };
        if generation == self.generation {
            return vec![];
        }
        if generation > self.generation
            || self
                .history
                .front()
                .is_none_or(|(g, _)| generation + 1 < *g)
        {
            return vec![(0, 0, w, h)];
        }
        compact(
            self.history
                .iter()
                .filter(|(g, _)| *g > generation)
                .flat_map(|(_, r)| r.iter().copied()),
            w,
            h,
        )
    }
    pub fn copy_regions(&self, canvas: &mut [u8], regions: &[Region]) {
        let stride = self.frame().width() as usize * 4;
        for &(x0, y0, x1, y1) in regions {
            for y in y0..y1 {
                let start = y as usize * stride + x0 as usize * 4;
                let end = y as usize * stride + x1 as usize * 4;
                for (src, dst) in self.frame().data()[start..end]
                    .chunks_exact(4)
                    .zip(canvas[start..end].chunks_exact_mut(4))
                {
                    dst.copy_from_slice(&[src[2], src[1], src[0], src[3]]);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn regional_frames_and_rotating_buffers_match_full_render() {
        for alpha in [0, 127, 255] {
            let mut base = Pixmap::new(640, 360).unwrap();
            for (i, pixel) in base.data_mut().chunks_exact_mut(4).enumerate() {
                let (x, y) = ((i % 640) as u32, (i / 640) as u32);
                pixel.copy_from_slice(&[
                    (x % 256 * u32::from(alpha) / 255) as u8,
                    (y % 256 * u32::from(alpha) / 255) as u8,
                    ((x + y) % 256 * u32::from(alpha) / 255) as u8,
                    alpha,
                ]);
            }
            let mut cache = SceneCache::new(&base);
            let mut reference = render::CachedOverlay::new(&base);
            let mut buffers: Vec<_> = (0..3).map(|_| (None, vec![0; 640 * 360 * 4])).collect();
            let mut seed = 73u32;
            for i in 0..160 {
                seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                let scene = Scene {
                    selection: (i % 17 != 0).then_some((
                        (seed % 570) as f32 - 20.25,
                        (seed % 300) as f32 - 10.5,
                        (seed % 450 + 1) as f32,
                        (seed % 230 + 1) as f32,
                    )),
                    editing: i % 3 != 0,
                    record: i % 5 < 2,
                    toggles: (i % 2 == 0, i % 3 == 0, i % 4 == 0),
                    hovered: Some(i % 4),
                    magnifier: (i % 5 == 3).then_some(((seed % 640) as f64, (seed % 360) as f64)),
                };
                assert!(cache.update(&base, scene.clone()));
                let expected = reference.reset(&base, scene.selection);
                paint(expected, &base, &scene);
                assert_eq!(
                    cache.frame().data(),
                    expected.data(),
                    "frame {i}, alpha {alpha}"
                );
                assert!(!cache.update(&base, scene));
                // Coalesce several states while all buffers are unavailable.
                if i % 7 < 2 {
                    continue;
                }
                let slot = &mut buffers[i % 3];
                // A buffer can miss more than the retained history on release.
                if i % 23 == 0 {
                    slot.0 = Some(0);
                }
                let damage = cache.damage_since(slot.0);
                cache.copy_regions(&mut slot.1, &damage);
                slot.0 = Some(cache.generation());
                let mut full = vec![0; slot.1.len()];
                render::pixmap_to_argb8888(expected, &mut full);
                assert_eq!(slot.1, full, "buffer {i}, alpha {alpha}");
            }
        }
    }
    #[test]
    fn small_drag_does_not_damage_selection_interior() {
        let base = Pixmap::new(3840, 2160).unwrap();
        let mut cache = SceneCache::new(&base);
        let mut scene = Scene {
            selection: Some((100., 100., 3000., 1600.)),
            editing: true,
            record: false,
            toggles: (false, false, false),
            hovered: None,
            magnifier: None,
        };
        cache.update(&base, scene.clone());
        let generation = cache.generation();
        scene.selection.as_mut().unwrap().0 += 1.;
        cache.update(&base, scene);
        let area: u64 = cache
            .damage_since(Some(generation))
            .iter()
            .map(|r| (r.2 - r.0) as u64 * (r.3 - r.1) as u64)
            .sum();
        assert!(area < 3840 * 2160 / 10, "{area}");
    }
}
