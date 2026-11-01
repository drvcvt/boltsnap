//! Real text-input-v3 wire peer and independent outbound request ledger.
use super::{Options, Request, State};
use wayland_protocols::wp::text_input::zv3::server::{
    zwp_text_input_manager_v3, zwp_text_input_v3,
};
use wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource, backend::GlobalId,
};
#[derive(Default)]
pub(in crate::window::tests) struct TextInputState {
    global: Option<GlobalId>,
    objects: Vec<zwp_text_input_v3::ZwpTextInputV3>,
}
#[derive(Debug)]
pub(in crate::window::tests) enum ImeWireEvent {
    Enter(usize, usize),
    Leave(usize, usize),
    Commit(usize, Option<String>),
    Preedit(usize, Option<String>, i32, i32),
    Delete(usize, u32, u32),
    Done(usize, u32),
    RemoveManager,
    AddManager,
}
pub(super) fn advertise(dh: &DisplayHandle, options: Options, state: &mut TextInputState) {
    if options.ime {
        state.global = Some(
            dh.create_global::<State, zwp_text_input_manager_v3::ZwpTextInputManagerV3, _>(1, ()),
        );
    }
}
impl GlobalDispatch<zwp_text_input_manager_v3::ZwpTextInputManagerV3, ()> for State {
    fn bind(
        state: &mut Self,
        _: &DisplayHandle,
        _: &Client,
        object: New<zwp_text_input_manager_v3::ZwpTextInputManagerV3>,
        _: &(),
        init: &mut DataInit<'_, Self>,
    ) {
        init.init(object, ());
        state.ledger.push(Request::BindIme);
    }
}
impl Dispatch<zwp_text_input_manager_v3::ZwpTextInputManagerV3, ()> for State {
    fn request(
        state: &mut Self,
        _: &Client,
        _: &zwp_text_input_manager_v3::ZwpTextInputManagerV3,
        request: zwp_text_input_manager_v3::Request,
        _: &(),
        _: &DisplayHandle,
        init: &mut DataInit<'_, Self>,
    ) {
        match request {
            zwp_text_input_manager_v3::Request::GetTextInput { id, seat } => {
                let seat = *seat.data::<usize>().unwrap();
                let index = state.ime.objects.len();
                state.ime.objects.push(init.init(id, index));
                state.ledger.push(Request::ImeObject(seat, index));
            }
            zwp_text_input_manager_v3::Request::Destroy => {
                state.ledger.push(Request::DestroyImeManager)
            }
            _ => unreachable!(),
        }
    }
}
impl Dispatch<zwp_text_input_v3::ZwpTextInputV3, usize> for State {
    fn request(
        state: &mut Self,
        _: &Client,
        _: &zwp_text_input_v3::ZwpTextInputV3,
        request: zwp_text_input_v3::Request,
        index: &usize,
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
        let index = *index;
        let request = match request {
            zwp_text_input_v3::Request::Enable => Request::ImeEnable(index),
            zwp_text_input_v3::Request::Disable => Request::ImeDisable(index),
            zwp_text_input_v3::Request::Commit => Request::ImeCommit(index),
            zwp_text_input_v3::Request::Destroy => Request::DestroyIme(index),
            zwp_text_input_v3::Request::SetSurroundingText {
                text,
                cursor,
                anchor,
            } => Request::ImeSurrounding(index, text, cursor, anchor),
            zwp_text_input_v3::Request::SetTextChangeCause { cause } => {
                Request::ImeCause(index, cause.into())
            }
            zwp_text_input_v3::Request::SetContentType { hint, purpose } => {
                Request::ImeContent(index, hint.into(), purpose.into())
            }
            zwp_text_input_v3::Request::SetCursorRectangle {
                x,
                y,
                width,
                height,
            } => Request::ImeCaret(index, x, y, width, height),
            _ => unreachable!(),
        };
        state.ledger.push(request);
    }
}
pub(super) fn emit(event: ImeWireEvent, state: &mut State, dh: &DisplayHandle) {
    let objects = &state.ime.objects;
    match event {
        ImeWireEvent::Enter(index, window) => objects[index].enter(&state.windows[window].surface),
        ImeWireEvent::Leave(index, window) => objects[index].leave(&state.windows[window].surface),
        ImeWireEvent::Commit(index, text) => objects[index].commit_string(text),
        ImeWireEvent::Preedit(index, text, begin, end) => {
            objects[index].preedit_string(text, begin, end)
        }
        ImeWireEvent::Delete(index, before, after) => {
            objects[index].delete_surrounding_text(before, after)
        }
        ImeWireEvent::Done(index, serial) => objects[index].done(serial),
        ImeWireEvent::AddManager => {
            state.ime.global = Some(
                dh.create_global::<State, zwp_text_input_manager_v3::ZwpTextInputManagerV3, _>(
                    1,
                    (),
                ),
            );
        }
        ImeWireEvent::RemoveManager => dh.remove_global::<State>(state.ime.global.clone().unwrap()),
    }
}
