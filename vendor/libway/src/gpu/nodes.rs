//! DRM node identity. The capture protocol forbids comparing `dev_t` values
//! directly, because a device may be named by its primary or its render node;
//! both are resolved to the set of nodes of the same GPU.
use std::path::{Path, PathBuf};

/// `(major, minor)` of every DRM node (card and render) that belongs to the
/// same device as `dev`, read from `sys`/dev/char/M:m/device/drm. Empty when
/// sysfs has no such device.
pub(crate) fn drm_nodes_in(sys: &Path, dev: u64) -> Vec<(u32, u32)> {
    let dir = sys
        .join("dev/char")
        .join(format!("{}:{}", libc::major(dev), libc::minor(dev)))
        .join("device/drm");
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut nodes: Vec<(u32, u32)> = entries
        .filter_map(|entry| {
            let text = std::fs::read_to_string(entry.ok()?.path().join("dev")).ok()?;
            let (major, minor) = text.trim().split_once(':')?;
            Some((major.parse().ok()?, minor.parse().ok()?))
        })
        .collect();
    nodes.sort_unstable();
    nodes
}

/// [`drm_nodes_in`] on the live `/sys`.
pub(crate) fn drm_nodes(dev: u64) -> Vec<(u32, u32)> {
    drm_nodes_in(Path::new("/sys"), dev)
}

/// The render node (minor 128 and up) among `nodes`, as its `/dev/dri` path.
pub(crate) fn render_node_of(nodes: &[(u32, u32)]) -> Option<PathBuf> {
    let minor = nodes
        .iter()
        .map(|(_, minor)| *minor)
        .find(|minor| *minor >= 128)?;
    Some(PathBuf::from(format!("/dev/dri/renderD{minor}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn primary_and_render_nodes_resolve_to_the_same_set() {
        let root = std::env::temp_dir().join(format!(
            "libway-drm-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let device = root.join("devices/pci0000:00/0000:01:00.0");
        for (name, dev) in [("card1", "226:1"), ("renderD128", "226:128")] {
            let node = device.join("drm").join(name);
            std::fs::create_dir_all(&node).unwrap();
            std::fs::write(node.join("dev"), format!("{dev}\n")).unwrap();
        }
        for link in ["226:1", "226:128"] {
            let char_dir = root.join("dev/char").join(link);
            std::fs::create_dir_all(&char_dir).unwrap();
            std::os::unix::fs::symlink(&device, char_dir.join("device")).unwrap();
        }
        let expected = vec![(226, 1), (226, 128)];
        assert_eq!(drm_nodes_in(&root, libc::makedev(226, 1)), expected);
        assert_eq!(drm_nodes_in(&root, libc::makedev(226, 128)), expected);
        assert!(drm_nodes_in(&root, libc::makedev(226, 2)).is_empty());
        assert_eq!(
            render_node_of(&expected),
            Some(PathBuf::from("/dev/dri/renderD128"))
        );
        assert_eq!(render_node_of(&[(226, 1)]), None);
        let _ = std::fs::remove_dir_all(root);
    }
}
