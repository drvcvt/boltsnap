//! The smooth cursor in a replay capture: leave out the system pointer,
//! load the gsr plugin and feed it from a tracker of each captured monitor
//! (one canvas for both, as recordings do with "Both displays").

use std::process::Command;

use super::monitor::Capture;

/// For smooth cursor modes, the trackers and their feed; they must live
/// until gsr is spawned (the trackers for the whole session).
pub(super) fn smooth(
    command: &mut Command,
    cursor: crate::config::RecordCursor,
    capture: &Capture,
) -> Result<
    Option<(
        Vec<super::super::cursor_track::Tracker>,
        super::super::cursor_track::Feeds,
    )>,
    String,
> {
    use super::super::cursor_track::{self, Feeds, Source};
    use crate::record::cursor_motion::{PLUGIN_ENV, PluginConfig, cursor_size};
    use std::os::fd::AsRawFd;
    use std::os::unix::process::CommandExt;
    if cursor == crate::config::RecordCursor::System {
        return Ok(None);
    }
    let plugin = cursor_track::plugin_path()?;
    let feeds = Feeds::new(1)?;
    let (trackers, origin, width) = match capture {
        Capture::One(monitor) => {
            let (tracker, area) =
                cursor_track::start(Source::Output(monitor.name.clone()), 0, None, &feeds)?;
            (vec![tracker], (area.x, area.y), area.width)
        }
        Capture::Both { monitors, .. } => {
            let trackers = monitors
                .iter()
                .enumerate()
                .map(|(index, m)| {
                    cursor_track::start(Source::Output(m.name.clone()), index, None, &feeds)
                        .map(|(tracker, _)| tracker)
                })
                .collect::<Result<Vec<_>, _>>()?;
            let (origin, width) = crate::record::session::logical_frame(monitors);
            (trackers, origin, width)
        }
    };
    let fd = feeds.readers[0].as_raw_fd();
    let config = PluginConfig {
        fd,
        preset: cursor.key().into(),
        size: cursor_size(),
        origin,
        width,
    };
    command
        .args(["-cursor", "no", "-p"])
        .arg(plugin)
        .env(PLUGIN_ENV, config.to_env());
    unsafe {
        command.pre_exec(move || {
            if libc::fcntl(fd, libc::F_SETFD, 0) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    Ok(Some((trackers, feeds)))
}
