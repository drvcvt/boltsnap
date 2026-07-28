pub mod capture;
pub mod clipboard;
pub mod ipc;
pub mod paths;
pub mod recording_codec;
pub mod select_skia;
pub mod shelf;
pub mod tray;

pub fn default_record_codec() -> String {
    recording_codec::auto_encoder().codec.clone()
}
