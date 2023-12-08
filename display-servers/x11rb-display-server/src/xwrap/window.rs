//! Xlib calls related to a window.

use leftwm_core::{
    models::{WindowChange, WindowHandle, WindowType, Xyhw},
    DisplayEvent, Window,
};
use x11rb::{protocol::xproto, x11_utils::Serialize};

use crate::xatom::WMStateWindowState;

use super::{XWrap, root_event_mask};

impl XWrap {
    /// Sets up a window before we manage it.
    pub fn setup_window(&self, window: xproto::Window) -> Option<DisplayEvent> {
        // Check that the window isn't requesting to be unmanaged
        let attrs = match self.get_window_attrs(window) {
            Ok(attr)
                if attr.override_redirect == false && !self.managed_windows.contains(&window) =>
            {
                attr
            }
            _ => return None,
        };
        let handle = WindowHandle::X11rbHandle(window);
        // Gather info about the window from xlib.
        let name = self.get_window_name(window);
        let legacy_name = self.get_window_legacy_name(window);
        let class = self.get_window_class(window);
        let pid = self.get_window_pid(window);
        let r#type = self.get_window_type(window);
        let states = self.get_window_states(window);
        let actions = self.get_window_actions_atoms(window);
        let mut can_resize = actions.contains(&self.atoms.NetWMActionResize);
        let trans = self.get_transient_for(window);
        let sizing_hint = self.get_hint_sizing_as_xyhw(window);
        let wm_hint = self.get_wmhints(window);

        // Build the new window, and fill in info about it.
        let mut w = Window::new(handle, name, pid);
        w.res_name = class
            .as_ref()
            .map(|c| String::from_utf8(c.instance().to_vec()).ok())
            .flatten();
        w.res_class = class
            .map(|c| String::from_utf8(c.class().to_vec()).ok())
            .flatten();
        w.legacy_name = legacy_name;
        w.r#type = r#type.clone();
        w.set_states(states);
        w.transient = trans.map(|h| WindowHandle::X11rbHandle(h));
        // // Initialise the windows floating with the pre-mapped settings.
        // let xyhw = XyhwChange {
        //     x: Some(attrs.x),
        //     y: Some(attrs.y),
        //     w: Some(attrs.width),
        //     h: Some(attrs.height),
        //     ..XyhwChange::default()
        // };
        // xyhw.update_window_floating(&mut w);
        sizing_hint
            .unwrap_or_default()
            .update_window_floating(&mut w);
        let mut requested = Xyhw::default();
        // xyhw.update(&mut requested);
        sizing_hint.unwrap_or_default().update(&mut requested);

        if let Some(hint) = sizing_hint {
            // Ignore this for now for non-splashes as it causes issues, e.g. mintstick is non-resizable but is too
            // small, issue #614: https://github.com/leftwm/leftwm/issues/614.
            can_resize = match (r#type, hint.minw, hint.minh, hint.maxw, hint.maxh) {
                (
                    WindowType::Splash,
                    Some(min_width),
                    Some(min_height),
                    Some(max_width),
                    Some(max_height),
                ) => can_resize || min_width != max_width || min_height != max_height,
                _ => true,
            };
            // Use the pre-mapped sizes if they are bigger.
            // hint.w = std::cmp::max(xyhw.w, hint.w);
            // hint.h = std::cmp::max(xyhw.h, hint.h);
            hint.update_window_floating(&mut w);
            hint.update(&mut requested);
        }

        w.requested = Some(requested);
        w.can_resize = can_resize;
        if let Some(hint) = wm_hint {
            w.never_focus = hint.input.unwrap_or(false);
            w.urgent = hint.urgent;
        }
        // Is this needed? Made it so it doens't overwrite prior sizing.
        if w.floating() && sizing_hint.is_none() {
            if let Ok(geo) = self.get_window_geometry(window) {
                geo.update_window_floating(&mut w);
            }
        }

        let cursor = self.get_cursor_point().unwrap_or_default();
        Some(DisplayEvent::WindowCreate(w, cursor.0, cursor.1))
    }

    /// Sets up a window that we want to manage.
    // `XMapWindow`: https://tronche.com/gui/x/xlib/window/XMapWindow.html
    pub fn setup_managed_window(
        &mut self,
        h: WindowHandle,
        floating: bool,
        follow_mouse: bool,
    ) -> Option<DisplayEvent> {
        let WindowHandle::X11rbHandle(handle) = h else {
            return None;
        };
        self.subscribe_to_window_events(handle);
        self.managed_windows.push(handle);

        // Make sure the window is mapped.
        xproto::map_window(&self.conn, handle).ok()?;

        // Let Xlib know we are managing this window.
        self.append_property_u32(
            self.root,
            self.atoms.NetClientList,
            xproto::AtomEnum::ATOM.into(),
            &[handle],
        );

        // Make sure there is at least an empty list of _NET_WM_STATE.
        let states = self.get_window_states_atoms(handle);
        self.set_window_states_atoms(handle, &states);

        // Set WM_STATE to normal state to allow window sharing.
        self.set_wm_state(handle, WMStateWindowState::Normal);

        let r#type = self.get_window_type(handle);
        if r#type == WindowType::Dock || r#type == WindowType::Desktop {
            if let Some(dock_area) = self.get_window_strut_array(handle) {
                let dems = self.get_screens_area_dimensions();
                let screen = self
                    .get_screens()
                    .iter()
                    .find(|s| s.contains_dock_area(dock_area, dems))?
                    .clone();

                if let Some(xyhw) = dock_area.as_xyhw(dems.0, dems.1, &screen) {
                    let mut change = WindowChange::new(h);
                    change.strut = Some(xyhw.into());
                    change.r#type = Some(r#type);
                    return Some(DisplayEvent::WindowChange(change));
                }
            } else if let Ok(geo) = self.get_window_geometry(handle) {
                let mut xyhw = Xyhw::default();
                geo.update(&mut xyhw);
                let mut change = WindowChange::new(h);
                change.strut = Some(xyhw.into());
                change.r#type = Some(r#type);
                return Some(DisplayEvent::WindowChange(change));
            }
        } else {
            let color = if floating {
                self.colors.floating
            } else {
                self.colors.normal
            };
            self.set_window_border_color(handle, color);

            if follow_mouse {
                _ = self.move_cursor_to_window(handle);
            }
            if self.focus_behaviour.is_clickto() {
                self.grab_mouse_clicks(handle, false);
            }
        }
        None
    }

    /// Teardown a managed window when it is destroyed.
    // `XGrabServer`: https://tronche.com/gui/x/xlib/window-and-session-manager/XGrabServer.html
    // `XUngrabServer`: https://tronche.com/gui/x/xlib/window-and-session-manager/XUngrabServer.html
    pub fn teardown_managed_window(&mut self, h: &WindowHandle, destroyed: bool) {
        if let WindowHandle::X11rbHandle(handle) = h {
            self.managed_windows.retain(|x| *x != *handle);
            if !destroyed {
                xproto::grab_server(&self.conn).unwrap();
                self.ungrab_buttons(*handle);
                self.set_wm_state(*handle, WMStateWindowState::Withdrawn);
                self.sync();
                xproto::ungrab_server(&self.conn).unwrap();
            }
            self.set_client_list();
        }
    }

    /// Updates a window.
    pub fn update_window(&self, window: &Window) {
        if let WindowHandle::X11rbHandle(handle) = window.handle {
            if window.visible() {
                let changes = xproto::ConfigureWindowAux {
                    x: Some(window.x()),
                    y: Some(window.y()),
                    width: Some(window.width() as u32),
                    height: Some(window.height() as u32),
                    border_width: Some(window.border() as u32),
                    ..Default::default()
                };
                self.set_window_config(handle, &changes);
                self.configure_window(window);
            }
            let Some((state, _)) = self.get_wm_state(handle) else {
                return;
            };
            // Only change when needed. This prevents task bar icons flashing (especially with steam).
            if window.visible() && state != WMStateWindowState::Normal {
                self.toggle_window_visibility(handle, true);
            } else if !window.visible() && state != WMStateWindowState::Iconic {
                self.toggle_window_visibility(handle, false);
            }
        }
    }

    /// Maps and unmaps a window depending on it is visible.
    pub fn toggle_window_visibility(&self, window: xproto::Window, visible: bool) {
        // We don't want to receive this map or unmap event.
        let mask_off = root_event_mask().remove(xproto::EventMask::SUBSTRUCTURE_NOTIFY);
        let mut attrs = xproto::ChangeWindowAttributesAux {
            event_mask: Some(mask_off),
            ..Default::default()
        };
        xproto::change_window_attributes(&self.conn, self.root, &attrs).unwrap();
        if visible {
            // Set WM_STATE to normal state.
            self.set_wm_state(window, WMStateWindowState::Normal);
            // Make sure the window is mapped.
            xproto::map_window(&self.conn, window).unwrap();
            // Regrab the mouse clicks but ignore `dock` windows as some don't handle click events put on them
            if self.focus_behaviour.is_clickto() && self.get_window_type(window) != WindowType::Dock
            {
                self.grab_mouse_clicks(window, false);
            }
        } else {
            // Ungrab the mouse clicks.
            self.ungrab_buttons(window);
            // Make sure the window is unmapped.
            xproto::unmap_window(&self.conn, window).unwrap();
            // Set WM_STATE to iconic state.
            self.set_wm_state(window, WMStateWindowState::Iconic);
        }
        attrs.event_mask = Some(root_event_mask());
        xproto::change_window_attributes(&self.conn, self.root, &attrs).unwrap();
    }

    /// Makes a window take focus.
    pub fn window_take_focus(&mut self, window: &Window, previous: Option<&Window>) {
        if let WindowHandle::X11rbHandle(handle) = window.handle {
            // Update previous window.
            if let Some(previous) = previous {
                if let WindowHandle::X11rbHandle(previous_handle) = previous.handle {
                    let color = if previous.floating() {
                        self.colors.floating
                    } else {
                        self.colors.normal
                    };
                    self.set_window_border_color(previous_handle, color);
                    // Open up button1 clicking on the previously focused window.
                    if self.focus_behaviour.is_clickto() {
                        self.grab_mouse_clicks(previous_handle, false);
                    }
                }
            }
            self.focused_window = handle;
            self.grab_mouse_clicks(handle, true);
            self.set_window_urgency(handle, false);
            self.set_window_border_color(handle, self.colors.active);
            self.focus(handle, window.never_focus);
            self.sync();
        }
    }

    /// Focuses a window.
    // `XSetInputFocus`: https://tronche.com/gui/x/xlib/input/XSetInputFocus.html
    pub fn focus(&mut self, window: xproto::Window, never_focus: bool) {
        if !never_focus {
            xproto::set_input_focus(
                &self.conn,
                xproto::InputFocus::POINTER_ROOT,
                window,
                x11rb::CURRENT_TIME,
            )
            .unwrap();
            self.replace_property_u32(
                window,
                self.atoms.NetActiveWindow,
                xproto::AtomEnum::ATOM.into(),
                &[window],
            );
        }
        // Tell the window to take focus
        self.send_xevent_atom(window, self.atoms.WMTakeFocus);
    }

    /// Unfocuses all windows.
    // `XSetInputFocus`: https://tronche.com/gui/x/xlib/input/XSetInputFocus.html
    pub fn unfocus(&self, handle: Option<WindowHandle>, floating: bool) {
        if let Some(WindowHandle::X11rbHandle(handle)) = handle {
            let color = if floating {
                self.colors.floating
            } else {
                self.colors.normal
            };
            self.set_window_border_color(handle, color);

            self.grab_mouse_clicks(handle, false);
        }
        xproto::set_input_focus(
            &self.conn,
            xproto::InputFocus::POINTER_ROOT,
            self.root,
            x11rb::CURRENT_TIME,
        )
        .unwrap();
        self.replace_property_u32(
            self.root,
            self.atoms.NetActiveWindow,
            xproto::AtomEnum::WINDOW.into(),
            &[x11rb::NONE],
        );
    }

    /// Send a `XConfigureEvent` for a window to X.
    pub fn configure_window(&self, window: &Window) {
        if let WindowHandle::X11rbHandle(handle) = window.handle {
            let configure_event = xproto::ConfigureNotifyEvent {
                event: handle,
                window: handle,
                x: window.x() as i16,
                y: window.y() as i16,
                width: window.width() as u16,
                height: window.height() as u16,
                border_width: window.border() as u16,
                above_sibling: x11rb::NONE,
                override_redirect: false,
                ..Default::default()
            };
            self.send_xevent(
                handle,
                false,
                xproto::EventMask::STRUCTURE_NOTIFY,
                &configure_event.serialize(),
            );
        }
    }

    /// Change a windows attributes.
    // `XChangeWindowAttributes`: https://tronche.com/gui/x/xlib/window/XChangeWindowAttributes.html
    // TODO: Is this method really useful ?
    pub fn change_window_attributes(
        &self,
        window: xproto::Window,
        attrs: &xproto::ChangeWindowAttributesAux,
    ) {
        xproto::change_window_attributes(&self.conn, window, attrs).unwrap();
    }

    /// Restacks the windows to the order of the vec.
    // `XRestackWindows`: https://tronche.com/gui/x/xlib/window/XRestackWindows.html
    pub fn restack(&self, handles: Vec<WindowHandle>) {
        let mut conf = xproto::ConfigureWindowAux::default();
        for i in 1..handles.len() {
            let Some(WindowHandle::X11rbHandle(window)) = handles.get(i) else {
                continue;
            };

            conf.stack_mode = Some(xproto::StackMode::BELOW);
            conf.sibling = handles
                .get(i - 1)
                .copied()
                .map(|h| {
                    if let WindowHandle::X11rbHandle(w) = h {
                        Some(w)
                    } else {
                        None
                    }
                })
                .flatten();
            xproto::configure_window(&self.conn, *window, &conf).unwrap();
        }
    }

    pub fn move_resize_window(&self, window: xproto::Window, x: i32, y: i32, w: u32, h: u32) {
        let attrs = xproto::ConfigureWindowAux {
            x: Some(x),
            y: Some(y),
            width: Some(w),
            height: Some(h),
            ..Default::default()
        };
        xproto::configure_window(&self.conn, window, &attrs).unwrap();
    }

    /// Raise a window.
    // `XRaiseWindow`: https://tronche.com/gui/x/xlib/window/XRaiseWindow.html
    pub fn move_to_top(&self, handle: &WindowHandle) {
        if let WindowHandle::X11rbHandle(window) = handle {
            let attrs = xproto::ConfigureWindowAux {
                stack_mode: Some(xproto::StackMode::ABOVE),
                ..Default::default()
            };
            xproto::configure_window(&self.conn, *window, &attrs).unwrap();
        }
    }

    /// Kills a window.
    // `XGrabServer`: https://tronche.com/gui/x/xlib/window-and-session-manager/XGrabServer.html
    // `XSetCloseDownMode`: https://tronche.com/gui/x/xlib/display/XSetCloseDownMode.html
    // `XKillClient`: https://tronche.com/gui/x/xlib/window-and-session-manager/XKillClient.html
    // `XUngrabServer`: https://tronche.com/gui/x/xlib/window-and-session-manager/XUngrabServer.html
    pub fn kill_window(&self, h: &WindowHandle) {
        if let WindowHandle::X11rbHandle(handle) = h {
            // Nicely ask the window to close.
            if !self.send_xevent_atom(*handle, self.atoms.WMDelete) {
                // Force kill the window.
                xproto::grab_server(&self.conn).unwrap();
                xproto::set_close_down_mode(&self.conn, xproto::CloseDown::DESTROY_ALL).unwrap();
                xproto::kill_client(&self.conn, *handle).unwrap();
                xproto::ungrab_server(&self.conn).unwrap();
            }
        }
    }

    /// Forcibly unmap a window.
    pub fn force_unmapped(&mut self, window: xproto::Window) {
        let managed = self.managed_windows.contains(&window);
        if managed {
            self.managed_windows.retain(|x| *x != window);
            self.set_client_list();
        }
    }

    /// Subscribe to an event of a window.
    // `XSelectInput`: https://tronche.com/gui/x/xlib/event-handling/XSelectInput.html
    pub fn subscribe_to_event(&self, window: xproto::Window, mask: xproto::EventMask) {
        // In xlib `XSelectInput` "lock" the display with `XLockDisplay` when setting the event
        // mask, is this needed here ?
        let attrs = xproto::ChangeWindowAttributesAux {
            event_mask: Some(mask),
            ..Default::default()
        };
        xproto::change_window_attributes(&self.conn, window, &attrs).unwrap();
    }

    /// Subscribe to the wanted events of a window.
    pub fn subscribe_to_window_events(&self, window: xproto::Window) {
        let mask = xproto::EventMask::ENTER_WINDOW
            | xproto::EventMask::FOCUS_CHANGE
            | xproto::EventMask::PROPERTY_CHANGE;
        self.subscribe_to_event(window, mask);
    }
}