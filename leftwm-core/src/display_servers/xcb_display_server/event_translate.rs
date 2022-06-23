use x11rb::protocol::{
    xproto::{
        self, ClientMessageEvent, ConfigureRequestEvent, ConfigureWindowAux, DestroyNotifyEvent,
        FocusInEvent, MapRequestEvent, ModMask, NotifyDetail, NotifyMode, PropertyNotifyEvent,
        UnmapNotifyEvent,
    },
    Event,
};

use crate::{
    models::{WindowChange, WindowType, XyhwChange},
    DisplayEvent, Mode, check_xcb_error,
};

use super::{
    event_translate_client_message, event_translate_property_notify,
    xcbwrap::{XCBWrap, WITHDRAWN_STATE},
};

pub struct XCBEvent<'a> {
    pub(crate) xcbw: &'a mut XCBWrap,
    pub(crate) event: Event,
}

impl<'a> From<XCBEvent<'a>> for Option<DisplayEvent> {
    fn from(event: XCBEvent) -> Self {
        let normal_mode = event.xcbw.mode == Mode::Normal;
        let sloppy_behaviour = event.xcbw.focus_behaviour.is_sloppy();

        match event.event {
            // New window is mapped.
            Event::MapRequest(ev) => from_map_request(event.xcbw, ev),
            // Window is unmapped.
            Event::UnmapNotify(ev) => from_unmap_event(event.xcbw, event.event, ev),
            // Window is destroyed.
            Event::DestroyNotify(ev) => from_destroy_notify(event.xcbw, ev),
            // Window is taking focus.
            Event::FocusIn(ev) => from_focus_in(event.xcbw, ev),
            // Window client message.
            Event::ClientMessage(ev) => from_client_message(event.xcbw, ev),
            // Window property notify.
            Event::PropertyNotify(ev) => from_property_notify(event.xcbw, ev),
            // Window configure request.
            Event::ConfigureRequest(ev) => from_configure_request(event.xcbw, ev),
            // Mouse entered notify.
            Event::EnterNotify(ev) if normal_mode && sloppy_behaviour => {
                from_enter_notify(event.xcbw, ev)
            }
            // Mouse motion notify.
            Event::MotionNotify(ev) => from_motion_notify(event.xcbw, ev),
            // Mouse button pressed.
            Event::ButtonPress(ev) => Some(from_button_press(ev)),
            // Mouse button released.
            Event::ButtonRelease(ev) if !normal_mode => Some(from_button_release(event.xcbw)),
            // Keyboard key pressed.
            Event::KeyPress(ev) => from_key_press(event.xcbw, ev),
            // Listen for keyboard changes.
            Event::MappingNotify(ev) => from_mapping_notify(event.xcbw, ev),
            _other => None,
        }
    }
}

fn from_map_request(xcbw: &mut XCBWrap, event: MapRequestEvent) -> Option<DisplayEvent> {
    check_xcb_error!(xcbw.setup_window(event.window), None)
}

fn from_unmap_event(
    xcbw: &mut XCBWrap,
    raw_event: Event,
    event: UnmapNotifyEvent,
) -> Option<DisplayEvent> {
    if xcbw.managed_windows.contains(&event.window) {
        if raw_event.sent_event() {
            let h = event.window.into();
            check_xcb_error!(xcbw.teardown_managed_window(&h, false), None);
            return Some(DisplayEvent::WindowDestroy(h));
        }
        // Set WM_STATE to withdrawn state.
        xcbw.set_wm_states(event.window, &[WITHDRAWN_STATE]);
    }
    None
}

fn from_destroy_notify(xcbw: &mut XCBWrap, event: DestroyNotifyEvent) -> Option<DisplayEvent> {
    if xcbw.managed_windows.contains(&event.window) {
        let h = event.window.into();
        check_xcb_error!(xcbw.teardown_managed_window(&h, true), None);
        return Some(DisplayEvent::WindowDestroy(h));
    }
    None
}

fn from_focus_in(xcbw: &mut XCBWrap, event: FocusInEvent) -> Option<DisplayEvent> {
    // Check that if a window is taking focus, that it should be.
    if xcbw.focused_window != event.event {
        let never_focus = !check_xcb_error!(xcbw.get_wmhints(xcbw.focused_window), None)
            .input
            .unwrap_or(true);
        check_xcb_error!(xcbw.focus(xcbw.focused_window, never_focus), None);
    }
    None
}

fn from_client_message(xcbw: &mut XCBWrap, event: ClientMessageEvent) -> Option<DisplayEvent> {
    check_xcb_error!(event_translate_client_message::from_event(xcbw, event), None)
}

fn from_property_notify(xcbw: &mut XCBWrap, event: PropertyNotifyEvent) -> Option<DisplayEvent> {
    check_xcb_error!(event_translate_property_notify::from_event(xcbw, event), None)
}

fn from_configure_request(
    xcbw: &mut XCBWrap,
    event: ConfigureRequestEvent,
) -> Option<DisplayEvent> {
    // If the window is not mapped, configure it.
    if !xcbw.managed_windows.contains(&event.window) {
        // let window_changes = xlib::XWindowChanges {
        //     x: event.x,
        //     y: event.y,
        //     width: event.width,
        //     height: event.height,
        //     border_width: event.border_width,
        //     sibling: event.above,
        //     stack_mode: event.detail,
        // };
        let window_changes = ConfigureWindowAux::new()
            .x(Some(event.x.into()))
            .y(Some(event.y.into()))
            .width(Some(event.width.into()))
            .height(Some(event.height.into()))
            .border_width(Some(event.border_width.into()))
            .sibling(Some(event.sibling))
            .stack_mode(Some(event.stack_mode));
        check_xcb_error!(xcbw.set_window_config(event.window, &window_changes), None);
        // This shouldn't be needed
        // xcbw.move_resize_window(
        //     event.window,
        //     event.x,
        //     event.y,
        //     event.width as u32,
        //     event.height as u32,
        // );
        return None;
    }
    let window_type = check_xcb_error!(xcbw.get_window_type(event.window), None);
    let trans = check_xcb_error!(xcbw.get_transient_for(event.window), None);
    let handle = event.window.into();
    if window_type == WindowType::Normal && trans.is_none() {
        return Some(DisplayEvent::ConfigureXlibWindow(handle));
    }
    let mut change = WindowChange::new(handle);
    let xyhw = match window_type {
        // We want to handle the window positioning when it is a dialog or a normal window with a
        // parent.
        WindowType::Dialog | WindowType::Normal => XyhwChange {
            w: Some(event.width.into()),
            h: Some(event.height.into()),
            ..XyhwChange::default()
        },
        _ => XyhwChange {
            w: Some(event.width.into()),
            h: Some(event.height.into()),
            x: Some(event.x.into()),
            y: Some(event.y.into()),
            ..XyhwChange::default()
        },
    };
    change.floating = Some(xyhw);
    Some(DisplayEvent::WindowChange(change))
}

fn from_enter_notify(xcbw: &mut XCBWrap, event: xproto::EnterNotifyEvent) -> Option<DisplayEvent> {
    if event.mode != NotifyMode::NORMAL
        || event.detail == NotifyDetail::INFERIOR
        || event.event == xcbw.get_default_root()
    {
        return None;
    }

    let h = event.event.into();
    Some(DisplayEvent::WindowTakeFocus(h))
}

fn from_motion_notify(
    xcbw: &mut XCBWrap,
    event: xproto::MotionNotifyEvent,
) -> Option<DisplayEvent> {
    // Limit motion events to current refresh rate.
    if xcbw.refresh_rate > 0
        && event.time as u64 - xcbw.motion_event_limiter > (1000 / xcbw.refresh_rate) as u64
    {
        xcbw.motion_event_limiter = event.time.into();
        let event_h = event.event.into();
        let offset_x = event.root_x as i32 - xcbw.mode_origin.0;
        let offset_y = event.root_y as i32 - xcbw.mode_origin.1;
        let display_event = match xcbw.mode {
            Mode::ReadyToMove(h) => {
                xcbw.set_mode(Mode::MovingWindow(h));
                DisplayEvent::MoveWindow(h, offset_x, offset_y)
            }
            Mode::MovingWindow(h) => DisplayEvent::MoveWindow(h, offset_x, offset_y),
            Mode::ReadyToResize(h) => {
                xcbw.set_mode(Mode::ResizingWindow(h));
                DisplayEvent::ResizeWindow(h, offset_x, offset_y)
            }
            Mode::ResizingWindow(h) => DisplayEvent::ResizeWindow(h, offset_x, offset_y),
            Mode::Normal if xcbw.focus_behaviour.is_sloppy() => {
                DisplayEvent::Movement(event_h, event.root_x.into(), event.root_y.into())
            }
            Mode::Normal => return None,
        };
        return Some(display_event);
    }

    None
}

fn from_button_press(event: xproto::ButtonPressEvent) -> DisplayEvent {
    let h = event.event.into();
    let mut mod_mask = event.state;
    mod_mask &= !(u16::from(ModMask::M2) | u16::from(ModMask::LOCK));
    DisplayEvent::MouseCombo(
        mod_mask.into(),
        event.detail.into(),
        h,
        event.event_x.into(),
        event.event_y.into(),
    )
}

fn from_button_release(xcbw: &mut XCBWrap) -> DisplayEvent {
    xcbw.set_mode(Mode::Normal);
    DisplayEvent::ChangeToNormalMode
}

fn from_key_press(xcbw: &mut XCBWrap, event: xproto::KeyPressEvent) -> Option<DisplayEvent> {
    let sym = check_xcb_error!(xcbw.keycode_to_keysym(event.detail), None);
    Some(DisplayEvent::KeyCombo(event.state.into(), sym))
}

fn from_mapping_notify(
    xcbw: &mut XCBWrap,
    event: xproto::MappingNotifyEvent,
) -> Option<DisplayEvent> {
    if event.request == xproto::Mapping::MODIFIER || event.request == xproto::Mapping::KEYBOARD {
        // Refresh keyboard.
        log::debug!("Updating keyboard");
        check_xcb_error!(xcbw.refresh_keyboard(event), None);

        // SoftReload keybinds.
        Some(DisplayEvent::KeyGrabReload)
    } else {
        None
    }
}
