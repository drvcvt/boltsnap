//! The grab press rule (D3) and the single grab chain (D4).
use super::*;

#[test]
fn a_grab_uses_the_latest_pointer_or_key_press_in_the_parent_tree() {
    let (mut fixture, toplevel, seat) = seated();
    refused(&mut fixture, grab(toplevel, seat), not_ready);
    click(&mut fixture, 0, 41);
    let (first, requests) = create(&mut fixture, grab(toplevel, seat));
    assert!(
        requests.ends_with(&[Request::Grab(1, 0, 41), Request::Commit(1, false)]),
        "{requests:?}"
    );
    close(&mut fixture, first);
    key(&mut fixture, 0, 42);
    let (second, requests) = create(&mut fixture, grab(toplevel, seat));
    assert!(requests.contains(&Request::Grab(2, 0, 42)), "{requests:?}");
    close(&mut fixture, second);
    // A press on another window does not authorize this tree.
    second_toplevel(&mut fixture, 3);
    click(&mut fixture, 3, 43);
    refused(&mut fixture, grab(toplevel, seat), not_ready);
}

#[test]
fn nested_grabs_use_the_root_press_and_a_closed_root_popup_still_counts() {
    let (mut fixture, toplevel, seat) = seated();
    click(&mut fixture, 0, 41);
    let menu_popup = shown(&mut fixture, grab(toplevel, seat), 1, PLACED);
    // A hover submenu: the latest press is on the toplevel, not the menu.
    let (_, requests) = create(&mut fixture, grab(menu_popup, seat));
    assert!(
        requests.contains(&Request::Popup(2, Some(1), 1)),
        "{requests:?}"
    );
    assert!(requests.contains(&Request::Grab(2, 0, 41)), "{requests:?}");
    close(&mut fixture, menu_popup);
    // wind Left/Right: a key on the root menu, which is then closed.
    let bar_menu = shown(&mut fixture, grab(toplevel, seat), 3, PLACED);
    key(&mut fixture, 3, 50);
    close(&mut fixture, bar_menu);
    let (next, requests) = create(&mut fixture, grab(toplevel, seat));
    assert!(requests.contains(&Request::Grab(4, 0, 50)), "{requests:?}");
    close(&mut fixture, next);
    let other = second_toplevel(&mut fixture, 5);
    refused(&mut fixture, grab(other, seat), not_ready);
}

#[test]
fn one_grab_chain_extends_only_from_its_topmost_grabbing_popup() {
    let (mut fixture, toplevel, seat) = seated();
    click(&mut fixture, 0, 41);
    let tooltip = shown(&mut fixture, menu(toplevel), 1, PLACED);
    refused(&mut fixture, grab(tooltip, seat), invalid);
    let root = shown(&mut fixture, grab(toplevel, seat), 2, PLACED);
    let (_, requests) = create(&mut fixture, grab(root, seat));
    assert!(requests.contains(&Request::Grab(3, 0, 41)), "{requests:?}");
    refused(&mut fixture, grab(root, seat), invalid);
    refused(&mut fixture, grab(toplevel, seat), invalid);
    let other = second_toplevel(&mut fixture, 4);
    click(&mut fixture, 4, 44);
    refused(&mut fixture, grab(other, seat), invalid);
    // Popups without a grab may sit anywhere while the chain is open.
    let (_, requests) = create(&mut fixture, menu(other));
    assert!(
        requests.contains(&Request::Popup(5, Some(4), 3)),
        "{requests:?}"
    );
}

#[test]
fn losing_the_device_or_the_seat_removes_the_grab_source() {
    let (mut fixture, toplevel, seat) = seated();
    click(&mut fixture, 0, 41);
    // Capability bits: pointer 1, keyboard 2. Keep only the keyboard.
    super::manage::send(&mut fixture, Input::Capabilities(0, 2));
    fixture.peer.take_requests();
    refused(&mut fixture, grab(toplevel, seat), not_ready);
    key(&mut fixture, 0, 42);
    // A key press survives the loss of a pointer the seat regained meanwhile.
    super::manage::send(&mut fixture, Input::Capabilities(0, 3));
    super::manage::send(&mut fixture, Input::Capabilities(0, 2));
    fixture.host.events();
    fixture.peer.take_requests();
    let (popup, requests) = create(&mut fixture, grab(toplevel, seat));
    assert!(requests.contains(&Request::Grab(1, 0, 42)), "{requests:?}");
    close(&mut fixture, popup);
    super::manage::send(&mut fixture, Input::RemoveSeat(0));
    fixture.host.events();
    fixture.peer.take_requests();
    refused(&mut fixture, grab(toplevel, seat), invalid);
}
