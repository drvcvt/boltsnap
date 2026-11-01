//! Proxy admission, host batches and partial terminal results have distinct outcomes.
use super::*;

#[test]
fn proxy_queue_caps_at_256_and_host_applies_exact_order_in_batches_of_64() {
    let mut fixture = Fixture::new();
    let id = fixture.create(640, 480);
    fixture.peer.take_requests();
    let proxy = fixture.host.proxy();
    for index in 0..256 {
        proxy
            .try_send(WindowCommand::SetTitle(id, index.to_string()))
            .unwrap();
    }
    let Err(CommandSendError::Full(WindowCommand::SetTitle(returned, title))) =
        proxy.try_send(WindowCommand::SetTitle(id, "unaccepted".into()))
    else {
        panic!("full admission must return the exact unaccepted command");
    };
    assert_eq!(returned, id);
    assert_eq!(title, "unaccepted");
    assert!(
        fixture.peer.requests().is_empty(),
        "admission does not apply commands"
    );
    for batch in 0..4 {
        let result = fixture.host.pump(Some(Instant::now() + WATCHDOG)).unwrap();
        assert_eq!(result.commands.len(), 64);
        assert_eq!(result.runnable, batch < 3);
        assert!(
            result
                .commands
                .iter()
                .all(|outcome| outcome.window == id && outcome.result.is_ok())
        );
    }
    let expected: Vec<_> = (0..256)
        .map(|index| Request::Title(0, index.to_string()))
        .collect();
    assert_eq!(fixture.peer.requests(), expected);
    let idle = fixture.host.pump(Some(Instant::now())).unwrap();
    assert!(idle.commands.is_empty() && !idle.runnable);
}

#[test]
fn invalid_titles_return_per_command_errors_without_replacing_or_stopping_the_window() {
    let mut fixture = Fixture::new();
    let id = fixture.create(640, 480);
    fixture.peer.take_requests();
    let proxy = fixture.host.proxy();
    for title in [
        "bad\0title".into(),
        "x".repeat(1025),
        "valid after refusal".into(),
    ] {
        proxy.try_send(WindowCommand::SetTitle(id, title)).unwrap();
    }
    let result = fixture.host.pump(Some(Instant::now() + WATCHDOG)).unwrap();
    assert_eq!(result.commands.len(), 3);
    assert!(matches!(
        result.commands[0].result,
        Err(Error::InvalidInput(_))
    ));
    assert!(matches!(
        result.commands[1].result,
        Err(Error::LimitExceeded)
    ));
    assert!(result.commands[2].result.is_ok());
    assert!(fixture.host.failure().is_none());
    assert_eq!(
        fixture.peer.requests(),
        [Request::Title(0, "valid after refusal".into())]
    );
    fixture.host.set_title(id, "still live".into()).unwrap();
    fixture.sync();
}

#[test]
fn terminal_event_overflow_retains_prior_command_outcomes_and_retires_leases() {
    let mut fixture = Fixture::new();
    let id = fixture.create(640, 480);
    let lease = fixture.host.lease(id).unwrap();
    let proxy = fixture.host.proxy();
    fixture.peer.take_requests();
    fixture
        .host
        .coordinator
        .state
        .events
        .extend(std::iter::repeat_n(
            WindowEvent::Ready,
            super::super::state::MAX_EVENTS,
        ));
    proxy
        .try_send(WindowCommand::SetTitle(id, "before failure".into()))
        .unwrap();
    proxy.try_send(WindowCommand::Close(id)).unwrap();
    let failure = fixture.host.pump(Some(Instant::now())).unwrap_err();
    assert_eq!(failure.commands.len(), 2);
    assert!(failure.commands[0].result.is_ok());
    assert!(matches!(
        failure.commands[1].result,
        Err(Error::LimitExceeded)
    ));
    assert!(lease.is_retired());
    assert!(matches!(
        proxy.try_send(WindowCommand::Close(id)),
        Err(CommandSendError::Closed(_))
    ));
    assert_eq!(fixture.host.events().len(), super::super::state::MAX_EVENTS);
    let first = fixture.host.failure().unwrap().to_owned();
    let later = fixture.host.pump(Some(Instant::now())).unwrap_err();
    assert!(later.commands.is_empty());
    assert!(matches!(later.error, Error::SessionStopped));
    assert_eq!(fixture.host.failure(), Some(first.as_str()));
    // The role remains owned by the explicit lease after the terminal host stop.
    assert!(
        !fixture
            .peer
            .requests()
            .iter()
            .any(|request| matches!(request, Request::DestroySurface(_)))
    );
    drop(lease);
}
