//! Pure helpers for area recording (geometry mapping + wf-recorder argv). The
//! live lifecycle (spawn, overlays, stop) lives in the shelf daemon.

use std::path::Path;

/// A recording region in compositor-global (logical) coordinates, as wf-recorder's
/// `-g` expects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Geometry {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
}

impl Geometry {
    /// wf-recorder `-g` string: "X,Y WxH".
    pub fn to_arg(&self) -> String {
        format!("{},{} {}x{}", self.x, self.y, self.w, self.h)
    }
}

/// Map a selection rect (logical surface px, origin = the overlay's output) to
/// global coords by adding the focused output's layout origin. Both are logical
/// (Hyprland monitor `x`/`y` are logical), so no scale division is needed.
pub fn to_global_geometry(
    rect_x: f64,
    rect_y: f64,
    rect_w: f64,
    rect_h: f64,
    output_x: i32,
    output_y: i32,
) -> Geometry {
    Geometry {
        x: output_x + rect_x.round() as i32,
        y: output_y + rect_y.round() as i32,
        w: rect_w.round().max(1.0) as u32,
        h: rect_h.round().max(1.0) as u32,
    }
}

/// Build the wf-recorder argv (excluding the program name) for a region recording.
pub fn wf_recorder_args(geo: &Geometry, codec: &str, out: &Path) -> Vec<String> {
    vec![
        "-g".into(),
        geo.to_arg(),
        "-c".into(),
        codec.into(),
        "-f".into(),
        out.to_string_lossy().into_owned(),
    ]
}

/// Build the wf-recorder argv (excluding the program name) to record an entire
/// output (monitor) by name — used for fullscreen recording.
pub fn wf_recorder_output_args(output: &str, codec: &str, out: &Path) -> Vec<String> {
    vec![
        "-o".into(),
        output.into(),
        "-c".into(),
        codec.into(),
        "-f".into(),
        out.to_string_lossy().into_owned(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn global_geometry_adds_output_origin() {
        let g = to_global_geometry(10.0, 20.0, 800.4, 600.6, 2560, 0);
        assert_eq!(
            g,
            Geometry {
                x: 2570,
                y: 20,
                w: 800,
                h: 601
            }
        );
        assert_eq!(g.to_arg(), "2570,20 800x601");
    }

    #[test]
    fn wf_args_shape() {
        let g = Geometry {
            x: 0,
            y: 0,
            w: 1280,
            h: 720,
        };
        let args = wf_recorder_args(&g, "h264_nvenc", &PathBuf::from("/tmp/r.mp4"));
        assert_eq!(
            args,
            vec!["-g", "0,0 1280x720", "-c", "h264_nvenc", "-f", "/tmp/r.mp4"]
        );
    }

    #[test]
    fn wf_output_args_shape() {
        let args = wf_recorder_output_args("DP-1", "h264_nvenc", &PathBuf::from("/tmp/r.mp4"));
        assert_eq!(
            args,
            vec!["-o", "DP-1", "-c", "h264_nvenc", "-f", "/tmp/r.mp4"]
        );
    }
}
