//! Crate-private capture paths against the isolated test server.
use super::*;
use crate::support::{Config, Server};

fn options() -> CaptureOptions {
    CaptureOptions {
        timeout: Duration::from_millis(500),
        ..Default::default()
    }
}

#[test]
fn a_consumer_failure_never_retries_the_batch_on_wlr() {
    let (server, socket) = Server::start(Config {
        outputs: 2,
        ..Default::default()
    });
    let opts = options();
    let mut c = Connection::from_socket(socket, &opts).unwrap();
    let outputs = c.outputs(&opts).unwrap();
    let end = Instant::now() + opts.timeout;
    let mut calls = 0;
    let result = c.capture_batch_each(&outputs, &opts, end, BufferKind::Cpu, &mut |_, _| {
        calls += 1;
        Err(Error::CaptureFailed("consumer".into()))
    });
    assert!(matches!(result, Err(Error::CaptureFailed(_))));
    assert_eq!(calls, 1, "a WLR retry would hand frames over again");
    c.outputs(&opts).unwrap();
    server.settle();
    assert_eq!(server.metrics.buffers.load(Ordering::SeqCst), 0);
}
