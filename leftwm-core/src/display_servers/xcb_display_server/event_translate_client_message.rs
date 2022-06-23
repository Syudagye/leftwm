use super::xcbwrap::{XCBResult, XCBWrap};
use super::DisplayEvent;
use crate::{models::WindowChange, Command};
use x11rb::protocol::xproto;

pub fn from_event(
    xcbw: &mut XCBWrap,
    event: xproto::ClientMessageEvent,
) -> XCBResult<Option<DisplayEvent>> {
    if !xcbw.managed_windows.contains(&event.window) && event.window != xcbw.get_default_root() {
        return Ok(None);
    }
    let atom_name = xcbw.get_xatom_name(event.type_)?;
    log::trace!("ClientMessage: {} : {:?}", event.window, atom_name);

    if event.type_ == xcbw.atoms._NET_CURRENT_DESKTOP {
        let index = event.data.as_data32()[0] as usize;
        let ev = DisplayEvent::SendCommand(Command::GoToTag {
            tag: index + 1,
            swap: false,
        });
        return Ok(Some(ev));

        // match usize::try_from(value) {
        //     Ok(index) => {
        //         let event = DisplayEvent::SendCommand(Command::GoToTag {
        //             tag: index + 1,
        //             swap: false,
        //         });
        //         return Some(event);
        //     }
        //     Err(err) => {
        //         log::debug!(
        //             "Received invalid value for current desktop new index ({}): {}",
        //             value,
        //             err,
        //         );
        //         return None;
        //     }
        // }
    }
    if event.type_ == xcbw.atoms._NET_WM_DESKTOP {
        let index = event.data.as_data32()[0] as usize;
        let ev = DisplayEvent::SendCommand(Command::SendWindowToTag {
            window: Some(event.window.into()),
            tag: index + 1,
        });
        return Ok(Some(ev));
        // let value = event.data.get_long(0);
        // match usize::try_from(value) {
        //     Ok(index) => {
        //         let event = DisplayEvent::SendCommand(Command::SendWindowToTag {
        //             window: Some(event.window.into()),
        //             tag: index + 1,
        //         });
        //         return Some(event);
        //     }
        //     Err(err) => {
        //         log::debug!(
        //             "Received invalid value for current desktop new index ({}): {}",
        //             value,
        //             err,
        //         );
        //         return None;
        //     }
        // }
    }
    if event.type_ == xcbw.atoms._NET_ACTIVE_WINDOW {
        xcbw.set_window_urgency(event.window, true)?;
        return Ok(None);
    }

    //if the client is trying to toggle fullscreen without changing the window state, change it too
    if event.type_ == xcbw.atoms._NET_WM_STATE
        && (event.data.as_data32()[1] == xcbw.atoms._NET_WM_STATE_FULLSCREEN
            || event.data.as_data32()[2] == xcbw.atoms._NET_WM_STATE_FULLSCREEN)
    {
        let set_fullscreen = event.data.as_data32()[0] == 1;
        let toggle_fullscreen = event.data.as_data32()[0] == 2;
        let mut states = xcbw.get_window_states_atoms(event.window)?;
        //determine what to change the state to
        let fullscreen = if toggle_fullscreen {
            !states.contains(&xcbw.atoms._NET_WM_STATE_FULLSCREEN)
        } else {
            set_fullscreen
        };
        //update the list of states
        if fullscreen {
            states.push(xcbw.atoms._NET_WM_STATE_FULLSCREEN);
        } else {
            states.retain(|x| x != &xcbw.atoms._NET_WM_STATE_FULLSCREEN);
        }
        states.sort_unstable();
        states.dedup();
        //set the windows state
        xcbw.set_window_states_atoms(event.window, &states)?;
    }

    //update the window states
    if event.type_ == xcbw.atoms._NET_WM_STATE {
        let handle = event.window.into();
        let mut change = WindowChange::new(handle);
        let states = xcbw.get_window_states(event.window)?;
        change.states = Some(states);
        return Ok(Some(DisplayEvent::WindowChange(change)));
    }

    Ok(None)
}
