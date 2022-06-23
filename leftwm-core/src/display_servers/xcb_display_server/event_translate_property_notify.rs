use super::{
    xcbwrap::{XCBResult, XCBWrap},
    DisplayEvent,
};
use crate::models::{WindowChange, WindowType, Xyhw};
use x11rb::protocol::xproto::{self, AtomEnum};

pub fn from_event(
    xcbw: &mut XCBWrap,
    event: xproto::PropertyNotifyEvent,
) -> XCBResult<Option<DisplayEvent>> {
    if event.window == xcbw.get_default_root()
        || event.state == xproto::Property::DELETE
        || !xcbw.managed_windows.contains(&event.window)
    {
        return Ok(None);
    }

    let event_name = xcbw.get_xatom_name(event.atom)?;
    log::trace!("PropertyNotify: {} : {:?}", event_name, &event);

    match AtomEnum::from(event.atom as u8) {
        AtomEnum::WM_TRANSIENT_FOR => {
            let window_type = xcbw.get_window_type(event.window)?;
            let handle = event.window.into();
            let mut change = WindowChange::new(handle);
            if window_type != WindowType::Normal {
                let trans = xcbw.get_transient_for(event.window)?;
                match trans {
                    Some(trans) => change.transient = Some(Some(trans.into())),
                    None => change.transient = Some(None),
                }
            }
            Ok(Some(DisplayEvent::WindowChange(change)))
        }
        AtomEnum::WM_NORMAL_HINTS => {
            Ok(build_change_for_size_hints(xcbw, event.window)?.map(DisplayEvent::WindowChange))
        }
        AtomEnum::WM_HINTS => {
            let hints = xcbw.get_wmhints(event.window)?;
            if hints.input == Some(true) {
                let handle = event.window.into();
                let mut change = WindowChange::new(handle);
                change.never_focus = Some(!hints.input.unwrap_or(true));
                return Ok(Some(DisplayEvent::WindowChange(change)));
            }
            Ok(None)
        }
        AtomEnum::WM_NAME => Ok(Some(update_title(xcbw, event.window)?)),
        _ => {
            if event.atom == xcbw.atoms._NET_WM_NAME {
                return Ok(Some(update_title(xcbw, event.window)?));
            }

            if event.atom == xcbw.atoms._NET_WM_STRUT
                || event.atom == xcbw.atoms._NET_WM_STRUT_PARTIAL
                    && xcbw.get_window_type(event.window)? == WindowType::Dock
            {
                if let Some(change) = build_change_for_size_strut_partial(xcbw, event.window)? {
                    return Ok(Some(DisplayEvent::WindowChange(change)));
                }
            }

            if event.atom == xcbw.atoms._NET_WM_STATE {
                let handle = event.window.into();
                let mut change = WindowChange::new(handle);
                let states = xcbw.get_window_states(event.window)?;
                change.states = Some(states);
                return Ok(Some(DisplayEvent::WindowChange(change)));
            }

            Ok(None)
        }
    }
}

fn build_change_for_size_strut_partial(
    xcbw: &XCBWrap,
    window: xproto::Window,
) -> XCBResult<Option<WindowChange>> {
    let handle = window.into();
    let mut change = WindowChange::new(handle);
    let r#type = xcbw.get_window_type(window)?;

    if let Some(dock_area) = xcbw.get_window_strut_array(window)? {
        let dems = xcbw.get_screens_area_dimensions()?;
        let screen = match xcbw
            .get_screens()?
            .iter()
            .find(|s| s.contains_dock_area(dock_area, dems))
        {
            None => return Ok(None),
            Some(v) => v.clone(),
        };

        if let Some(xyhw) = dock_area.as_xyhw(dems.0, dems.1, &screen) {
            change.floating = Some(xyhw.into());
            change.r#type = Some(r#type);
            return Ok(Some(change));
        }
    } else {
        let geo = xcbw.get_window_geometry(window)?;
        let mut xyhw = Xyhw::default();
        geo.update(&mut xyhw);
        change.floating = Some(xyhw.into());
        change.r#type = Some(r#type);
        return Ok(Some(change));
    }
    Ok(None)
}

fn build_change_for_size_hints(
    xcbw: &XCBWrap,
    window: xproto::Window,
) -> XCBResult<Option<WindowChange>> {
    let handle = window.into();
    let mut change = WindowChange::new(handle);
    let hint = xcbw.get_hint_sizing_as_xyhw(window)?;
    if hint.x.is_none() && hint.y.is_none() && hint.w.is_none() && hint.h.is_none() {
        //junk hint; change change anything
        return Ok(None);
    }
    let mut xyhw = Xyhw::default();
    hint.update(&mut xyhw);
    change.requested = Some(xyhw);
    Ok(Some(change))
}

fn update_title(xcbw: &XCBWrap, window: xproto::Window) -> XCBResult<DisplayEvent> {
    let title = xcbw.get_window_name(window)?;
    let handle = window.into();
    let mut change = WindowChange::new(handle);
    change.name = Some(Some(title));
    Ok(DisplayEvent::WindowChange(change))
}
