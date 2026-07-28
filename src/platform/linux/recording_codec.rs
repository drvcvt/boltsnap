use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;

const AMD_VENDOR_ID: &str = "0x1002";
const NVIDIA_VENDOR_ID: &str = "0x10de";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GpuVendor {
    Amd,
    Nvidia,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct GpuDevice {
    vendor: GpuVendor,
    render_node: Option<PathBuf>,
    primary: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EncoderChoice {
    pub codec: String,
    pub device: Option<PathBuf>,
    pub description: &'static str,
}

static AUTO_ENCODER: OnceLock<EncoderChoice> = OnceLock::new();

/// Detect the first hardware encoder that is both backed by a matching GPU and
/// can encode a real frame. The primary DRM GPU wins on hybrid systems.
pub fn auto_encoder() -> &'static EncoderChoice {
    AUTO_ENCODER.get_or_init(|| {
        let devices = discover_gpu_devices(Path::new("/sys/class/drm"), Path::new("/dev/dri"));
        select_encoder(&devices, |codec, device| {
            probe_encoder(Path::new("ffmpeg"), codec, device)
        })
    })
}

/// Return the VA-API render node for an automatically or explicitly selected
/// VA-API codec. NVENC and software encoders do not use wf-recorder's `-d`.
pub fn device_for_codec(codec: &str) -> Option<PathBuf> {
    if !codec.ends_with("_vaapi") {
        return None;
    }
    let detected = auto_encoder();
    if detected.codec == codec {
        return detected.device.clone();
    }
    let mut devices = discover_gpu_devices(Path::new("/sys/class/drm"), Path::new("/dev/dri"));
    devices.sort_by_key(|device| !device.primary);
    devices
        .into_iter()
        .find(|device| device.vendor == GpuVendor::Amd)
        .and_then(|device| device.render_node)
}

fn select_encoder(
    devices: &[GpuDevice],
    mut supported: impl FnMut(&str, Option<&Path>) -> bool,
) -> EncoderChoice {
    let mut ordered = devices.to_vec();
    ordered.sort_by_key(|device| !device.primary);

    for device in ordered {
        let (codec, encoder_device, description) = match device.vendor {
            GpuVendor::Nvidia => ("h264_nvenc", None, "NVIDIA NVENC"),
            GpuVendor::Amd if device.render_node.is_some() => {
                ("h264_vaapi", device.render_node.as_deref(), "AMD VA-API")
            }
            GpuVendor::Amd => continue,
        };
        if supported(codec, encoder_device) {
            return EncoderChoice {
                codec: codec.to_string(),
                device: encoder_device.map(Path::to_path_buf),
                description,
            };
        }
    }

    EncoderChoice {
        codec: "libx264".to_string(),
        device: None,
        description: "software x264",
    }
}

fn discover_gpu_devices(sys_class_drm: &Path, dev_dri: &Path) -> Vec<GpuDevice> {
    let mut devices = Vec::new();
    let Ok(entries) = std::fs::read_dir(sys_class_drm) else {
        return nvidia_device_fallback(devices);
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !name.starts_with("renderD") {
            continue;
        }
        let device_dir = entry.path().join("device");
        let Ok(vendor) = std::fs::read_to_string(device_dir.join("vendor")) else {
            continue;
        };
        let vendor = match vendor.trim().to_ascii_lowercase().as_str() {
            AMD_VENDOR_ID => GpuVendor::Amd,
            NVIDIA_VENDOR_ID => GpuVendor::Nvidia,
            _ => continue,
        };
        let primary = std::fs::read_to_string(device_dir.join("boot_vga"))
            .is_ok_and(|value| value.trim() == "1");
        devices.push(GpuDevice {
            vendor,
            render_node: Some(dev_dri.join(name.as_ref())),
            primary,
        });
    }
    nvidia_device_fallback(devices)
}

fn nvidia_device_fallback(mut devices: Vec<GpuDevice>) -> Vec<GpuDevice> {
    if devices
        .iter()
        .all(|device| device.vendor != GpuVendor::Nvidia)
        && Path::new("/dev/nvidia0").exists()
    {
        devices.push(GpuDevice {
            vendor: GpuVendor::Nvidia,
            render_node: None,
            primary: false,
        });
    }
    devices
}

fn probe_encoder(ffmpeg: &Path, codec: &str, device: Option<&Path>) -> bool {
    let mut command = Command::new(ffmpeg);
    command.args([
        "-hide_banner",
        "-loglevel",
        "error",
        "-f",
        "lavfi",
        "-i",
        "color=c=black:s=128x128:r=1",
    ]);
    if codec.ends_with("_vaapi") {
        let Some(device) = device else {
            return false;
        };
        command
            .arg("-vaapi_device")
            .arg(device)
            .args(["-vf", "format=nv12,hwupload"]);
    }
    command
        .args(["-frames:v", "1", "-c:v", codec, "-f", "null", "-"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gpu(vendor: GpuVendor, primary: bool, render_node: Option<&str>) -> GpuDevice {
        GpuDevice {
            vendor,
            primary,
            render_node: render_node.map(PathBuf::from),
        }
    }

    #[test]
    fn primary_amd_wins_on_a_hybrid_system() {
        let devices = vec![
            gpu(GpuVendor::Nvidia, false, Some("/dev/dri/renderD129")),
            gpu(GpuVendor::Amd, true, Some("/dev/dri/renderD128")),
        ];
        let choice = select_encoder(&devices, |_, _| true);
        assert_eq!(choice.codec, "h264_vaapi");
        assert_eq!(choice.device, Some("/dev/dri/renderD128".into()));
    }

    #[test]
    fn unavailable_primary_encoder_falls_through_to_other_gpu() {
        let devices = vec![
            gpu(GpuVendor::Amd, true, Some("/dev/dri/renderD128")),
            gpu(GpuVendor::Nvidia, false, None),
        ];
        let choice = select_encoder(&devices, |codec, _| codec == "h264_nvenc");
        assert_eq!(choice.codec, "h264_nvenc");
        assert_eq!(choice.device, None);
    }

    #[test]
    fn nvenc_does_not_receive_a_vaapi_render_node() {
        let devices = vec![gpu(GpuVendor::Nvidia, true, Some("/dev/dri/renderD128"))];
        let choice = select_encoder(&devices, |_, _| true);
        assert_eq!(choice.codec, "h264_nvenc");
        assert_eq!(choice.device, None);
    }

    #[test]
    fn missing_hardware_encoder_uses_x264() {
        let devices = vec![gpu(GpuVendor::Amd, true, Some("/dev/dri/renderD128"))];
        let choice = select_encoder(&devices, |_, _| false);
        assert_eq!(choice.codec, "libx264");
        assert_eq!(choice.description, "software x264");
    }
}
