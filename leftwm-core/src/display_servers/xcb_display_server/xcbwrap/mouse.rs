//! Xlib calls related to a mouse.
use crate::{BUTTONMASK, MOUSEMASK};

use super::{XCBError, XCBResult, XCBWrap};
use x11_dl::xlib;
use x11rb::{
    protocol::xproto::{
        self, allow_events, grab_button, grab_pointer, query_pointer, ungrab_button,
        ungrab_pointer, warp_pointer, Allow, AtomEnum, ButtonIndex, ButtonPressEvent,
        ClientMessageData, ClientMessageEvent, EventMask, ModMask, Window, BUTTON_PRESS_EVENT,
        BUTTON_RELEASE_EVENT,
    },
    CURRENT_TIME,
};

impl XCBWrap {
    /// Grabs the mouse clicks of a window.
    pub fn grab_mouse_clicks(&self, handle: xproto::Window, is_focused: bool) -> XCBResult<()> {
        self.ungrab_buttons(handle)?;
        if !is_focused {
            self.grab_buttons(handle, xproto::ButtonIndex::M1, xproto::ModMask::ANY)?;
            self.grab_buttons(handle, xproto::ButtonIndex::M3, xproto::ModMask::ANY)?;
        }
        let mouse_key_mask_with_shift = xproto::ModMask::from(
            u16::from(self.mouse_key_mask) | u16::from(xproto::ModMask::SHIFT),
        );
        self.grab_buttons(handle, xproto::ButtonIndex::M1, self.mouse_key_mask)?;
        self.grab_buttons(handle, xproto::ButtonIndex::M1, mouse_key_mask_with_shift)?;
        self.grab_buttons(handle, xproto::ButtonIndex::M3, self.mouse_key_mask)?;
        self.grab_buttons(handle, xproto::ButtonIndex::M3, mouse_key_mask_with_shift)
    }

    /// Grabs the button with the modifier for a window.
    // `XGrabButton`: https://tronche.com/gui/x/xlib/input/XGrabButton.html
    pub fn grab_buttons(
        &self,
        window: xproto::Window,
        button: xproto::ButtonIndex,
        modifiers: xproto::ModMask,
    ) -> XCBResult<()> {
        // Grab the buttons with and without numlock (Mod2).
        let mods: Vec<xproto::ModMask> = vec![
            modifiers,
            xproto::ModMask::from(u16::from(modifiers) | u16::from(xproto::ModMask::M2)),
            xproto::ModMask::from(u16::from(modifiers) | u16::from(xproto::ModMask::LOCK)),
        ];
        for m in mods {
            xproto::grab_button(
                &self.connection,
                false,
                window,
                u32::from(BUTTONMASK!()) as u16,
                xproto::GrabMode::ASYNC,
                xproto::GrabMode::ASYNC,
                0u32,
                0u32,
                button,
                m,
            )?;
        }
        Ok(())
    }

    /// Cleans all currently grabbed buttons of a window.
    // `XUngrabButton`: https://tronche.com/gui/x/xlib/input/XUngrabButton.html
    pub fn ungrab_buttons(&self, handle: xproto::Window) -> XCBResult<()> {
        // Assuming again from xlib that the ANY button is 0
        xproto::ungrab_button(
            &self.connection,
            xproto::ButtonIndex::ANY,
            handle,
            ModMask::ANY,
        )?;
        Ok(())
    }

    /// Grabs the cursor and sets its visual.
    // `XGrabPointer`: https://tronche.com/gui/x/xlib/input/XGrabPointer.html
    pub fn grab_pointer(
        &self,
        cursor: xproto::Cursor,
        // ) -> core::result::Result<(), xproto::GrabStatus> {
    ) -> XCBResult<()> {
        let reply = xproto::grab_pointer(
            &self.connection,
            false,
            self.root,
            u32::from(MOUSEMASK!()) as u16,
            xproto::GrabMode::ASYNC,
            xproto::GrabMode::ASYNC,
            0u16,
            cursor as u16,
            CURRENT_TIME,
        )?
        .reply()?;
        match reply.status {
            xproto::GrabStatus::SUCCESS => Ok(()),
            xproto::GrabStatus::FROZEN => Err(Box::new(XCBError(
                "An error occured when grabbing the pointer: Pointer Frozen",
            ))),
            xproto::GrabStatus::INVALID_TIME => Err(Box::new(XCBError(
                "An error occured when grabbing the pointer: Invalid Time",
            ))),
            xproto::GrabStatus::NOT_VIEWABLE => Err(Box::new(XCBError(
                "An error occured when grabbing the pointer: Pointer not viewable",
            ))),
            xproto::GrabStatus::ALREADY_GRABBED => Err(Box::new(XCBError(
                "An error occured when grabbing the pointer: Pointer already grabbed",
            ))),
            _ => Err(Box::new(XCBError(
                "An error occured when grabbing the pointer",
            ))),

        }
        // unsafe {
        //     //grab the mouse
        //     (self.xlib.XGrabPointer)(
        //         self.display,
        //         self.root,
        //         0,
        //         MOUSEMASK as u32,
        //         xlib::GrabModeAsync,
        //         xlib::GrabModeAsync,
        //         0,
        //         cursor,
        //         xlib::CurrentTime,
        //     );
        // }
    }

    /// Ungrab the cursor.
    // `XUngrabPointer`: https://tronche.com/gui/x/xlib/input/XUngrabPointer.html
    pub fn ungrab_pointer(&self) -> XCBResult<()> {
        xproto::ungrab_pointer(&self.connection, CURRENT_TIME)?;
        Ok(())
        // unsafe {
        //     //release the mouse grab
        //     (self.xlib.XUngrabPointer)(self.display, xlib::CurrentTime);
        // }
    }

    /// Move the cursor to a window.
    /// # Errors
    ///
    /// Will error if unable to obtain window attributes. See `get_window_attrs`.
    pub fn move_cursor_to_window(&self, window: xproto::Window) -> XCBResult<()> {
        let attrs = self.get_window_attrs(window)?;
        let geo = self.get_window_geometry(window)?;
        let point = (
            geo.x.unwrap_or_default() + (geo.w.unwrap_or_default() / 2),
            geo.y.unwrap_or_default() + (geo.h.unwrap_or_default() / 2),
        );
        self.move_cursor_to_point(point)
    }

    /// Move the cursor to a point.
    /// # Errors
    ///
    /// Error indicates `XlibError`.
    // `XWarpPointer`: https://tronche.com/gui/x/xlib/input/XWarpPointer.html
    // TODO: Verify that Error is unreachable or specify conditions that may result
    // in an error.
    pub fn move_cursor_to_point(&self, point: (i32, i32)) -> XCBResult<()> {
        if point.0 >= 0 && point.1 >= 0 {
            // let none: c_int = 0;
            xproto::warp_pointer(
                &self.connection,
                self.root,
                0u32,
                0,
                0,
                0,
                0,
                point.0 as i16,
                point.1 as i16,
            )?;
            // unsafe {
            //     (self.xlib.XWarpPointer)(
            //         self.display,
            //         none as c_ulong,
            //         self.root,
            //         none,
            //         none,
            //         none as u32,
            //         none as u32,
            //         point.0,
            //         point.1,
            //     );
            // }
        }
        Ok(())
    }

    /// Replay a click on a window.
    // `XQueryPointer`: https://tronche.com/gui/x/xlib/window-information/XQueryPointer.html
    pub fn replay_click(
        &self,
        focused_window: xproto::Window,
        button: xproto::Button,
    ) -> XCBResult<()> {
        unsafe {
            let mut ev: xproto::ButtonPressEvent = std::mem::zeroed();
            ev.detail = button;
            ev.same_screen = true;
            ev.child = self.get_default_root();

            // let mut event: xlib::XButtonEvent = std::mem::zeroed();
            // event.button = button;
            // event.same_screen = xlib::True;
            // event.subwindow = self.get_default_root();

            while ev.child != 0 {
                ev.event = ev.child;
                let res = query_pointer(&self.connection, ev.event)?.reply()?;
                ev.root = res.root;
                ev.child = res.child;
                ev.root_x = res.root_x;
                ev.root_y = res.root_y;
                ev.event_x = res.win_x;
                ev.event_y = res.win_y;
                ev.state = res.mask;

                // event.window = event.subwindow;
                // (self.xlib.XQueryPointer)(
                //     self.display,
                //     event.window,
                //     &mut event.root,
                //     &mut event.subwindow,
                //     &mut event.x_root,
                //     &mut event.y_root,
                //     &mut event.x,
                //     &mut event.y,
                //     &mut event.state,
                // );
            }

            // Make sure we are clicking on the focused window. This also prevents clicks when
            // focus is changed by a keybind.
            if ev.event == focused_window {
                // event.type_ = xlib::ButtonPress;
                ev.response_type = xproto::BUTTON_PRESS_EVENT;
                xproto::send_event(
                    &self.connection,
                    false,
                    ev.event,
                    xproto::EventMask::BUTTON_PRESS,
                    ev,
                )?;
                // self.send_xevent(ev.event, false, EventMask::BUTTON_PRESS, &mut ev.into())?;

                // event.type_ = xlib::ButtonRelease;
                ev.response_type = xproto::BUTTON_RELEASE_EVENT;
                xproto::send_event(
                    &self.connection,
                    false,
                    ev.event,
                    xproto::EventMask::BUTTON_RELEASE,
                    ev,
                )?;
                // self.send_xevent(ev.event, false, EventMask::BUTTON_RELEASE, &mut ev.into())?;
            }
            Ok(())
        }
    }

    /// Release the pointer if it is frozen.
    // `XAllowEvents`: https://linux.die.net/man/3/xallowevents
    pub fn allow_pointer_events(&self) -> XCBResult<()> {
        xproto::allow_events(&self.connection, Allow::SYNC_POINTER, CURRENT_TIME)?;
        Ok(())
    }
}
