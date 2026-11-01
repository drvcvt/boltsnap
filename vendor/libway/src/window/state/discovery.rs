//! Complete the asynchronous required-global snapshot.
use super::*;

impl WindowState {
    pub(super) fn begin_discovery(&mut self) {
        let registry = self.connection.display().get_registry(&self.qh, ());
        self.bootstrap = Some(Bootstrap {
            registry: Some(registry),
            compositor: None,
            shell: None,
            shm: None,
            scaling: ScaleGlobals::default(),
            decoration: Default::default(),
            layer_shell: Default::default(),
            attention: Default::default(),
            connection: self.connection.clone(),
            notifier: self.notifier.clone(),
        });
        self.connection.display().sync(&self.qh, ());
        if let Some(error) = self.connection.backend().last_error() {
            self.remember(&Error::Wayland(error.to_string()));
        }
    }

    pub(in crate::window) fn discovered(&mut self) {
        if !self.active() || self.globals.is_some() {
            return;
        }
        let Some(mut bootstrap) = self.bootstrap.take() else {
            return;
        };
        let Some((compositor_name, compositor)) = bootstrap.compositor.take() else {
            self.remember(&Error::Unsupported(
                "wl_compositor and xdg_wm_base are required",
            ));
            return;
        };
        let Some((shell_name, shell)) = bootstrap.shell.take() else {
            self.remember(&Error::Unsupported(
                "wl_compositor and xdg_wm_base are required",
            ));
            return;
        };
        // new stores the registry before publishing Bootstrap; discovery is its
        // sole removal site, and taking Bootstrap makes this transfer exclusive.
        self.globals = Some(Arc::new(Globals {
            connection: self.connection.clone(),
            _registry: bootstrap
                .registry
                .take()
                .expect("invariant: bootstrap owns registry"),
            compositor,
            shell,
            shm: bootstrap.shm.take(),
            scaling: std::mem::take(&mut bootstrap.scaling),
            decoration: std::mem::take(&mut bootstrap.decoration),
            layer_shell: std::mem::take(&mut bootstrap.layer_shell),
            attention: std::mem::take(&mut bootstrap.attention),
            compositor_name,
            shell_name,
            native_windows: AtomicUsize::new(0),
            notifier: self.notifier.clone(),
        }));
        self.push(WindowEvent::Ready);
    }
}
