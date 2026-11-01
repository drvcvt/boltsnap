//! Native attempt traces use the existing isolated peer, never a fabricated commit.
mod admission;
mod attempts;
mod presentation;

use super::*;

fn prepared(fixture: &mut Fixture) -> (WindowId, NativePresenter, Configuration) {
    let id = fixture.create(2, 1);
    let configuration = fixture.configure(id, 0, 2, 1);
    let claim = fixture.host.claim_native(id).unwrap();
    fixture.peer.take_requests();
    (id, claim, configuration)
}

fn frame<'a>(
    host: &'a mut Host,
    claim: &'a NativePresenter,
    configuration: Configuration,
) -> NativeFrame<'a> {
    match host
        .begin_native_frame(claim, configuration.revision)
        .unwrap()
    {
        NativeFrameStart::Ready(frame) => frame,
        state => panic!("expected native frame readiness, got {state:?}"),
    }
}

fn assert_stage(requests: Vec<Request>) {
    assert_eq!(requests, [Request::BufferScale(0, 1), Request::Frame(0, 0)]);
}
