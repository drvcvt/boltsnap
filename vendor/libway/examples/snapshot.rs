//! Explicit invocation captures the Wayland session selected by the environment.
use libway::{CaptureOptions, Connection};
use std::{fs::OpenOptions, io::BufWriter};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args_os()
        .nth(1)
        .ok_or("usage: snapshot OUTPUT.png")?;
    let options = CaptureOptions::default();
    let mut connection = Connection::connect(&options)?;
    let desktop = connection.capture_desktop(&options)?;
    let file = OpenOptions::new().write(true).create_new(true).open(path)?;
    let mut writer = BufWriter::new(file);
    desktop
        .image
        .write_to(&mut writer, image::ImageFormat::Png)?;
    use std::io::Write;
    writer.flush()?;
    Ok(())
}
