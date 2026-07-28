pub mod audio;
pub mod autostart;
pub mod capture;
pub mod clipboard;
pub mod hotkey;
pub mod ipc;
pub mod paths;
pub mod recording;
pub mod select_skia;
pub mod shelf;
pub mod tray;

/// Windows Media Foundation receives H.264 settings and selects the installed
/// AMD or NVIDIA hardware transform through MediaTranscoder.
pub fn default_record_codec() -> String {
    "h264".to_string()
}

pub(crate) fn app_window_icon() -> winit::window::Icon {
    let image =
        image::load_from_memory(include_bytes!("../../../assets/icons/boltsnap-app-64.png"))
            .expect("embedded Boltsnap app icon must be valid PNG")
            .into_rgba8();
    let (width, height) = image.dimensions();
    winit::window::Icon::from_rgba(image.into_raw(), width, height)
        .expect("embedded Boltsnap app icon must have valid RGBA dimensions")
}
