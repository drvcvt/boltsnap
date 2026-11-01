//! Polling peer and acknowledged command execution.
use super::*;
pub(super) fn run(socket: UnixStream, options: Options, mut mailbox: Mailbox<Command>) {
    let mut display = Display::<State>::new().unwrap();
    let mut dh = display.handle();
    dh.insert_client(socket, std::sync::Arc::new(ClientState))
        .unwrap();
    let output_globals = roles::advertise(&dh, options);
    let mut state = State {
        output_globals,
        hold_output_done: !options.output_done,
        shm_abgr: options.shm_abgr,
        ..State::default()
    };
    input::advertise(&dh, options, &mut state.input);
    clipboard::advertise(&dh, options, &mut state.clipboard);
    text_input::advertise(&dh, options, &mut state.ime);
    decoration::advertise(&dh, options, &mut state.decoration);
    cursor::advertise(&dh, options, &mut state.cursor);
    presentation::advertise(&dh, options, &mut state.presentation);
    primary::advertise(&dh, options, &mut state.primary);
    layer::advertise(&dh, options, &mut state.layer);
    toplevel::advertise(&dh, options, &mut state.attention);
    loop {
        mailbox.drain_wake().unwrap();
        display.dispatch_clients(&mut state).unwrap();
        while let Some(command) = mailbox.pop().unwrap() {
            // The caller flushed before control admission. Dispatch again after
            // receipt, so a snapshot never races an earlier empty dispatch.
            display.dispatch_clients(&mut state).unwrap();
            let stopped = command.apply(|command| apply(command, &mut display, &mut state));
            if stopped {
                return;
            }
        }
        display.flush_clients().unwrap();
        let mut fds = [
            libc::pollfd {
                fd: display.as_fd().as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            },
            libc::pollfd {
                fd: mailbox.wake_fd().as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            },
        ];
        // SAFETY: display and mailbox retain both fds throughout this poll; the
        // initialized array is writable for two entries and Stop durably wakes it.
        let result = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as _, -1) };
        if result < 0 {
            assert_eq!(
                std::io::Error::last_os_error().kind(),
                std::io::ErrorKind::Interrupted
            );
        }
    }
}

fn apply(command: Command, display: &mut Display<State>, state: &mut State) -> bool {
    let done = match command {
        Command::Configure {
            window,
            width,
            height,
            serial,
            states,
            done,
        } => {
            let native = &state.windows[window];
            native
                .toplevel
                .as_ref()
                .unwrap()
                .configure(width, height, states);
            native.role.as_ref().unwrap().configure(serial);
            Some(done)
        }
        Command::Close { window, done } => {
            state.windows[window].toplevel.as_ref().unwrap().close();
            Some(done)
        }
        Command::Snapshot { clear, done } => {
            let requests = if clear {
                std::mem::take(&mut state.ledger)
            } else {
                state.ledger.clone()
            };
            display.flush_clients().unwrap();
            done.send(requests).unwrap();
            None
        }
        Command::Stop => return true,
        Command::Emit { event, done } => {
            match event {
                Event::Ime(event) => text_input::emit(event, state, &display.handle()),
                Event::Input(event) => input::emit(event, state, &display.handle()),
                Event::Clipboard(event) => clipboard::emit(event, state, &display.handle()),
                Event::RemoveDecorationManager => {
                    decoration::remove_manager(state, &display.handle())
                }
                Event::RemoveCursorManager => cursor::remove_manager(state, &display.handle()),
                Event::Presentation(event) => presentation::emit(event, state, &display.handle()),
                Event::Primary(event) => primary::emit(event, state, &display.handle()),
                Event::Layer(event) => layer::emit(event, state, &display.handle()),
                Event::Activation(event) => {
                    toplevel::emit_activation(event, state, &display.handle())
                }
                Event::IdleInhibit(event) => {
                    toplevel::emit_idle_inhibit(event, state, &display.handle())
                }
                Event::RemoveOutput(index) => {
                    let global = state.output_globals[index].take().unwrap();
                    display.handle().remove_global::<State>(global);
                }
                other => emit(other, state),
            }
            Some(done)
        }
        Command::Buffers { done } => {
            done.send(buffer_snapshots(state)).unwrap();
            None
        }
    };
    display.flush_clients().unwrap();
    if let Some(done) = done {
        done.send(()).unwrap();
    }
    false
}

fn buffer_snapshots(state: &State) -> Vec<BufferSnapshot> {
    state
        .buffers
        .iter()
        .enumerate()
        .map(|(index, buffer)| {
            let length = (buffer.stride * buffer.height) as usize;
            BufferSnapshot {
                index,
                format: buffer.format,
                width: buffer.width,
                height: buffer.height,
                stride: buffer.stride,
                bytes: buffer.map[buffer.offset..buffer.offset + length].to_vec(),
            }
        })
        .collect()
}

fn emit(event: Event, state: &mut State) {
    match event {
        Event::Ime(_) | Event::Input(_) | Event::Clipboard(_) => {
            unreachable!("handled by protocol peer")
        }
        Event::Decoration(window, mode) => decoration::emit(state, window, mode),
        Event::Popup(event) => roles::popup::emit(event, state),
        Event::RemoveDecorationManager
        | Event::RemoveCursorManager
        | Event::Presentation(_)
        | Event::Primary(_)
        | Event::RemoveOutput(_)
        | Event::Layer(_)
        | Event::Activation(_)
        | Event::IdleInhibit(_) => {
            unreachable!("handled with the display handle")
        }
        Event::Release(index) => state.buffers[index].proxy.release(),
        Event::Frame(window) => {
            let index = state
                .frames
                .iter()
                .position(|(id, _)| *id == window)
                .unwrap();
            state.frames.remove(index).1.done(1);
        }
        Event::Fractional(window, scale) => state.windows[window]
            .fractional
            .as_ref()
            .unwrap()
            .preferred_scale(scale),
        Event::ShmFormat(format) => {
            let format = wayland_server::protocol::wl_shm::Format::try_from(format).unwrap();
            if format == wayland_server::protocol::wl_shm::Format::Abgr8888 {
                state.shm_abgr = true;
            }
            state.shm.as_ref().unwrap().format(format);
        }
        Event::Preferred(window, scale) => {
            state.windows[window].surface.preferred_buffer_scale(scale)
        }
        Event::Output(index, scale, done) => {
            let output = state.outputs[index].as_ref().unwrap();
            output.scale(scale);
            if done {
                output.done();
            }
        }
        Event::Enter(window, output) => state.windows[window]
            .surface
            .enter(state.outputs[output].as_ref().unwrap()),
        Event::Leave(window, output) => state.windows[window]
            .surface
            .leave(state.outputs[output].as_ref().unwrap()),
    }
}

struct ClientState;
impl ClientData for ClientState {
    fn initialized(&self, _: wayland_server::backend::ClientId) {}
    fn disconnected(
        &self,
        _: wayland_server::backend::ClientId,
        _: wayland_server::backend::DisconnectReason,
    ) {
    }
}
