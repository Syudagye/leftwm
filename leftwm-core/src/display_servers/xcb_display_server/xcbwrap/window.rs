//! Xlib calls related to a window.
use super::{
    Window, WindowHandle, XCBResult, XCBWrap, ICONIC_STATE, NORMAL_STATE, WITHDRAWN_STATE,
};
use crate::models::{WindowChange, WindowType, Xyhw};
use crate::{DisplayEvent, ROOT_EVENT_MASK};
use x11rb::protocol::xproto::{
    self, change_window_attributes, AtomEnum, ConfigureNotifyEvent, ConfigureWindowAux,
    ConnectionExt, EventMask, InputFocus, StackMode,
};
use x11rb::CURRENT_TIME;

impl XCBWrap {
    /// Sets up a window before we manage it.
    pub fn setup_window(&self, window: xproto::Window) -> XCBResult<Option<DisplayEvent>> {
        // Check that the window isn't requesting to be unmanaged
        let attrs = self.get_window_attrs(window)?;
        let geometry = self.get_window_geometry(window)?;
        if !attrs.override_redirect || self.managed_windows.contains(&window) {
            return Ok(None);
        }
        let handle = window.into();
        // Gather info about the window from xlib.
        let name = self.get_window_name(window)?;
        let legacy_name = self.get_window_legacy_name(window).unwrap_or_default();
        let class = self.get_window_class(window)?;
        let pid = self.get_window_pid(window)?;
        let r#type = self.get_window_type(window)?;
        let states = self.get_window_states(window)?;
        let actions = self.get_window_actions_atoms(window)?;
        let mut can_resize = actions.contains(&self.atoms._NET_WM_ACTION_RESIZE);
        let trans = self.get_transient_for(window)?;
        let mut sizing_hint = self.get_hint_sizing_as_xyhw(window)?;
        let wm_hint = self.get_wmhints(window)?;

        // Build the new window, and fill in info about it.
        let mut w = Window::new(handle, Some(name), Some(pid));
        w.res_name = String::from_utf8(class.instance().to_vec()).ok();
        w.res_class = String::from_utf8(class.class().to_vec()).ok();
        w.legacy_name = Some(legacy_name);
        w.r#type = r#type.clone();
        w.set_states(states);
        if let Some(trans) = trans {
            w.transient = Some(trans.into());
        }
        // Initialise the windows floating with the pre-mapped settings.
        let mut xyhw = geometry.clone();
        xyhw.maxw = sizing_hint.maxw;
        xyhw.maxh = sizing_hint.maxh;
        xyhw.update_window_floating(&mut w);
        let mut requested = Xyhw::default();
        xyhw.update(&mut requested);

        // Ignore this for now for non-splashes as it causes issues, e.g. mintstick is non-resizable but is too
        // small, issue #614: https://github.com/leftwm/leftwm/issues/614.
        can_resize = match (
            r#type,
            sizing_hint.minw,
            sizing_hint.minh,
            sizing_hint.maxw,
            sizing_hint.maxh,
        ) {
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
        sizing_hint.w = std::cmp::max(xyhw.w, sizing_hint.w);
        sizing_hint.h = std::cmp::max(xyhw.h, sizing_hint.h);
        sizing_hint.update_window_floating(&mut w);
        sizing_hint.update(&mut requested);

        w.requested = Some(requested);
        w.can_resize = can_resize;
        w.never_focus = !wm_hint.input.unwrap_or(true); // Not 100% sure, need to check

        // // Is this needed? Made it so it doens't overwrite prior sizing.
        // if w.floating() && sizing_hint.is_none() {
        //     if let Ok(geo) = self.get_window_geometry(window) {
        //         geo.update_window_floating(&mut w);
        //     }
        // }

        let cursor = self.get_cursor_point().unwrap_or_default();
        Ok(Some(DisplayEvent::WindowCreate(w, cursor.0, cursor.1)))
    }

    /// Sets up a window that we want to manage.
    // `XMapWindow`: https://tronche.com/gui/x/xlib/window/XMapWindow.html
    pub fn setup_managed_window(
        &mut self,
        h: WindowHandle,
        floating: bool,
        follow_mouse: bool,
    ) -> XCBResult<Option<DisplayEvent>> {
        let handle = match h.xcb_handle() {
            Some(h) => h,
            None => return Ok(None),
        };
        self.subscribe_to_window_events(handle)?;
        self.managed_windows.push(handle);

        // Make sure the window is mapped.
        xproto::map_window(&self.connection, handle)?;

        // Let X know we are managing this window.
        let list = handle.to_be_bytes();
        self.append_property_long(
            self.root,
            self.atoms._NET_CLIENT_LIST,
            AtomEnum::WINDOW.into(),
            &list,
        );

        // Make sure there is at least an empty list of _NET_WM_STATE.
        let states = self.get_window_states_atoms(handle)?;
        self.set_window_states_atoms(handle, &states)?;
        // Set WM_STATE to normal state to allow window sharing.
        self.set_wm_states(handle, &[NORMAL_STATE])?;

        let r#type = self.get_window_type(handle)?;
        if r#type == WindowType::Dock || r#type == WindowType::Desktop {
            if let Some(dock_area) = self.get_window_strut_array(handle)? {
                let dems = self.get_screens_area_dimensions()?;
                let screens = self.get_screens()?;
                let screen = screens
                    .iter()
                    .find(|s| s.contains_dock_area(dock_area, dems))
                    .clone();
                if let Some(screen) = screen {
                    if let Some(xyhw) = dock_area.as_xyhw(dems.0, dems.1, &screen) {
                        let mut change = WindowChange::new(h);
                        change.strut = Some(xyhw.into());
                        change.r#type = Some(r#type);
                        return Ok(Some(DisplayEvent::WindowChange(change)));
                    }
                }
            } else if let Ok(geo) = self.get_window_geometry(handle) {
                let mut xyhw = Xyhw::default();
                geo.update(&mut xyhw);
                let mut change = WindowChange::new(h);
                change.strut = Some(xyhw.into());
                change.r#type = Some(r#type);
                return Ok(Some(DisplayEvent::WindowChange(change)));
            }
        } else {
            let color = if floating {
                self.colors.floating
            } else {
                self.colors.normal
            };
            self.set_window_border_color(handle, color)?;

            if follow_mouse {
                let _ = self.move_cursor_to_window(handle)?;
            }
            if self.focus_behaviour.is_clickto() {
                self.grab_mouse_clicks(handle, false);
            }
        }
        Ok(None)
    }

    /// Teardown a managed window when it is destroyed.
    // `XGrabServer`: https://tronche.com/gui/x/xlib/window-and-session-manager/XGrabServer.html
    // `XUngrabServer`: https://tronche.com/gui/x/xlib/window-and-session-manager/XUngrabServer.html
    pub fn teardown_managed_window(&mut self, h: &WindowHandle, destroyed: bool) -> XCBResult<()> {
        if let WindowHandle::XCBHandle(handle) = h {
            self.managed_windows.retain(|x| *x != *handle);
            if !destroyed {
                xproto::grab_server(&self.connection)?;
                self.ungrab_buttons(*handle)?;
                self.set_wm_states(*handle, &[WITHDRAWN_STATE])?;
                self.sync()?;
                xproto::ungrab_server(&self.connection)?;
                // unsafe {
                //     (self.xlib.XGrabServer)(self.display);
                //     (self.xlib.XSetErrorHandler)(Some(on_error_from_xlib_dummy));
                //     self.ungrab_buttons(*handle);
                //     self.set_wm_states(*handle, &[WITHDRAWN_STATE]);
                //     self.sync();
                //     (self.xlib.XSetErrorHandler)(Some(on_error_from_xlib));
                //     (self.xlib.XUngrabServer)(self.display);
                // }
            }
            self.set_client_list();
        }
        Ok(())
    }

    /// Updates a window.
    pub fn update_window(&self, window: &Window) -> XCBResult<()> {
        if let WindowHandle::XCBHandle(handle) = window.handle {
            if window.visible() {
                let changes = ConfigureWindowAux::new()
                    .x(window.x())
                    .y(window.y())
                    .width(Some(window.width().try_into()?))
                    .height(Some(window.height().try_into()?))
                    .border_width(Some(window.border().try_into()?))
                    .sibling(0)
                    .stack_mode(Some(StackMode::ABOVE.into()));
                self.set_window_config(handle, &changes)?;
                // self.configure_window(window)?; // might me useless
            }
            let state = match self.get_wm_state(handle.into())? {
                Some(state) => state,
                None => return Ok(()),
            };
            // Only change when needed. This prevents task bar icons flashing (especially with steam).
            if window.visible() && state != NORMAL_STATE as u32 {
                self.toggle_window_visibility(handle, true)?;
            } else if !window.visible() && state != ICONIC_STATE as u32 {
                self.toggle_window_visibility(handle, false)?;
            }
        }
        Ok(())
    }

    /// Maps and unmaps a window depending on it is visible.
    pub fn toggle_window_visibility(&self, window: xproto::Window, visible: bool) -> XCBResult<()> {
        // We don't want to receive this map or unmap event.
        let mask_off = EventMask::from(
            u32::from(ROOT_EVENT_MASK!()) & !u32::from(xproto::EventMask::SUBSTRUCTURE_NOTIFY),
        );
        let mut attrs = xproto::ChangeWindowAttributesAux::new().event_mask(mask_off);
        self.change_window_attributes(self.root, attrs)?;
        if visible {
            // Set WM_STATE to normal state.
            self.set_wm_states(window, &[NORMAL_STATE])?;
            // Make sure the window is mapped.
            xproto::map_window(&self.connection, window)?;
            // Regrab the mouse clicks.
            if self.focus_behaviour.is_clickto() {
                self.grab_mouse_clicks(window, false)?;
            }
        } else {
            // Ungrab the mouse clicks.
            self.ungrab_buttons(window)?;
            // Make sure the window is unmapped.
            xproto::unmap_window(&self.connection, window)?;
            // Set WM_STATE to iconic state.
            self.set_wm_states(window, &[ICONIC_STATE])?;
        }
        attrs.event_mask = Some(ROOT_EVENT_MASK!().into());
        self.change_window_attributes(self.root, attrs)
    }

    /// Makes a window take focus.
    pub fn window_take_focus(
        &mut self,
        window: &Window,
        previous: Option<&Window>,
    ) -> XCBResult<()> {
        if let WindowHandle::XCBHandle(handle) = window.handle {
            // Update previous window.
            if let Some(previous) = previous {
                if let WindowHandle::XCBHandle(previous_handle) = previous.handle {
                    let color = if previous.floating() {
                        self.colors.floating
                    } else {
                        self.colors.normal
                    };
                    self.set_window_border_color(previous_handle, color)?;
                    // Open up button1 clicking on the previously focused window.
                    if self.focus_behaviour.is_clickto() {
                        self.grab_mouse_clicks(previous_handle, false)?;
                    }
                }
            }
            self.focused_window = handle;
            self.grab_mouse_clicks(handle, true)?;
            self.set_window_urgency(handle, false)?;
            self.set_window_border_color(handle, self.colors.active)?;
            self.focus(handle, window.never_focus)?;
            self.sync()?;
        }
        Ok(())
    }

    /// Focuses a window.
    // `XSetInputFocus`: https://tronche.com/gui/x/xlib/input/XSetInputFocus.html
    pub fn focus(&mut self, window: xproto::Window, never_focus: bool) -> XCBResult<()> {
        if !never_focus {
            xproto::set_input_focus(
                &self.connection,
                InputFocus::POINTER_ROOT,
                window,
                CURRENT_TIME,
            )?;
            self.replace_property_long(
                self.root,
                self.atoms._NET_ACTIVE_WINDOW,
                AtomEnum::WINDOW.into(),
                &window.to_ne_bytes(),
            )?;
        }
        // Tell the window to take focus
        self.send_xevent_atom(window, self.atoms.WM_TAKE_FOCUS)?;
        Ok(())
    }

    /// Unfocuses all windows.
    // `XSetInputFocus`: https://tronche.com/gui/x/xlib/input/XSetInputFocus.html
    pub fn unfocus(&self, handle: Option<WindowHandle>, floating: bool) -> XCBResult<()> {
        if let Some(WindowHandle::XCBHandle(handle)) = handle {
            let color = if floating {
                self.colors.floating
            } else {
                self.colors.normal
            };
            self.set_window_border_color(handle, color)?;

            self.grab_mouse_clicks(handle, false)?;
        }
        xproto::set_input_focus(
            &self.connection,
            xproto::InputFocus::POINTER_ROOT,
            self.root,
            x11rb::CURRENT_TIME,
        )?;
        self.replace_property_long(
            self.root,
            self.atoms._NET_ACTIVE_WINDOW,
            AtomEnum::WINDOW.into(),
            &[u8::MAX],
        )
    }

    /// Send a `XConfigureEvent` for a window to X.
    pub fn configure_window(&self, window: &Window) -> XCBResult<()> {
        if let WindowHandle::XCBHandle(handle) = window.handle {
            // let mut configure_event: xlib::XConfigureEvent = unsafe { std::mem::zeroed() };
            // configure_event.type_ = xlib::ConfigureNotify;
            // configure_event.display = self.display;
            // configure_event.event = handle;
            // configure_event.window = handle;
            // configure_event.x = window.x();
            // configure_event.y = window.y();
            // configure_event.width = window.width();
            // configure_event.height = window.height();
            // configure_event.border_width = window.border;
            // configure_event.above = 0;
            // configure_event.override_redirect = 0;

            let mut config: ConfigureNotifyEvent = unsafe { std::mem::zeroed() };
            config.window = handle;
            config.event = handle;
            config.x = window.x() as i16;
            config.y = window.y() as i16;
            config.width = window.width() as u16;
            config.height = window.height() as u16;
            config.border_width = window.border as u16;
            config.override_redirect = false;

            xproto::send_event(
                &self.connection,
                false,
                handle,
                EventMask::STRUCTURE_NOTIFY,
                config,
            )?;
            // self.send_xevent(
            //     handle,
            //     false,
            //     EventMask::STRUCTURE_NOTIFY,
            //     &mut configure_event.into(),
            // )?;
        }
        Ok(())
    }

    /// Change a windows attributes.
    // `XChangeWindowAttributes`: https://tronche.com/gui/x/xlib/window/XChangeWindowAttributes.html
    pub fn change_window_attributes(
        &self,
        window: xproto::Window,
        attrs: xproto::ChangeWindowAttributesAux,
    ) -> XCBResult<()> {
        xproto::change_window_attributes(&self.connection, window, &attrs)?;
        Ok(())
    }

    /// Restacks the windows to the order of the vec.
    // `XRestackWindows`: https://tronche.com/gui/x/xlib/window/XRestackWindows.html
    // https://stackoverflow.com/questions/60612753/how-to-restack-all-windows-properly-in-xcb-like-xrestackwindows-in-xlib
    pub fn restack(&self, handles: Vec<WindowHandle>) -> XCBResult<()> {
        for h in handles {
            let h = h.xcb_handle();
            if let Some(h) = h {
                let cfg = ConfigureWindowAux::new()
                    .sibling(h - 1)
                    .stack_mode(Some(xproto::StackMode::BELOW));
                xproto::configure_window(&self.connection, h, &cfg)?;
            }
        }
        Ok(())
        //
        // let mut windows = vec![];
        // for handle in handles {
        //     if let WindowHandle::XCBHandle(window) = handle {
        //         windows.push(window);
        //     }
        // }
        // let size = windows.len();
        // let ptr = windows.as_mut_ptr();
        // unsafe {
        //     (self.xlib.XRestackWindows)(self.display, ptr, size as i32);
        // }
    }

    // pub fn move_resize_window(&self, window: xlib::Window, x: i32, y: i32, w: u32, h: u32) {
    //     // TODO: Find a way to implement XMoveResizeWindow with xcb
    //     unsafe {
    //         (self.xlib.XMoveResizeWindow)(self.display, window, x, y, w, h);
    //     }
    // }

    /// Raise a window.
    // `XRaiseWindow`: https://tronche.com/gui/x/xlib/window/XRaiseWindow.html
    pub fn move_to_top(&self, handle: &WindowHandle) -> XCBResult<()> {
        if let WindowHandle::XCBHandle(window) = handle {
            return self.set_window_config(
                *window,
                &ConfigureWindowAux::new().stack_mode(Some(xproto::StackMode::ABOVE)),
            );
        }
        Ok(())
        // TODO: Find a way to implement XRaiseWindow with xcb
        // if let WindowHandle::XlibHandle(window) = handle {
        //     unsafe {
        //         (self.xlib.XRaiseWindow)(self.display, *window);
        //     }
        // }
    }

    /// Kills a window.
    // `XGrabServer`: https://tronche.com/gui/x/xlib/window-and-session-manager/XGrabServer.html
    // `XSetCloseDownMode`: https://tronche.com/gui/x/xlib/display/XSetCloseDownMode.html
    // `XKillClient`: https://tronche.com/gui/x/xlib/window-and-session-manager/XKillClient.html
    // `XUngrabServer`: https://tronche.com/gui/x/xlib/window-and-session-manager/XUngrabServer.html
    pub fn kill_window(&self, h: &WindowHandle) -> XCBResult<()> {
        if let WindowHandle::XCBHandle(handle) = h {
            // Nicely ask the window to close.
            if !self.send_xevent_atom(*handle, self.atoms.WM_DELETE)? {
                // Force kill the window.
                xproto::grab_server(&self.connection)?;
                // TODO: XSetCloseDownMode to xcb
                self.connection.kill_client(*handle)?;
                self.sync()?;
                xproto::ungrab_server(&self.connection)?;
                // unsafe {
                //     (self.xlib.XGrabServer)(self.display);
                //     (self.xlib.XSetErrorHandler)(Some(on_error_from_xlib_dummy));
                //     (self.xlib.XSetCloseDownMode)(self.display, xlib::DestroyAll);
                //     (self.xlib.XKillClient)(self.display, *handle);
                //     self.sync();
                //     (self.xlib.XSetErrorHandler)(Some(on_error_from_xlib));
                //     (self.xlib.XUngrabServer)(self.display);
                // }
            }
        }
        Ok(())
    }

    /// Forcibly unmap a window.
    pub fn force_unmapped(&mut self, window: xproto::Window) -> XCBResult<()> {
        let managed = self.managed_windows.contains(&window);
        if managed {
            self.managed_windows.retain(|x| *x != window);
            self.set_client_list()?;
        }
        Ok(())
    }

    /// Subscribe to an event of a window.
    // `XSelectInput`: https://tronche.com/gui/x/xlib/event-handling/XSelectInput.html
    pub fn subscribe_to_event(&self, window: xproto::Window, mask: EventMask) -> XCBResult<()> {
        change_window_attributes(
            &self.connection,
            window,
            &xproto::ChangeWindowAttributesAux::new().event_mask(mask),
        )?;
        Ok(())
        // unsafe { (self.xlib.XSelectInput)(self.display, window, mask) };
    }

    /// Subscribe to the wanted events of a window.
    pub fn subscribe_to_window_events(&self, window: xproto::Window) -> XCBResult<()> {
        let mask = EventMask::ENTER_WINDOW | EventMask::FOCUS_CHANGE | EventMask::PROPERTY_CHANGE;
        self.subscribe_to_event(window, mask)
    }
}
