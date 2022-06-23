//! `XWrap` getters.
use std::intrinsics::transmute;

use super::{WindowHandle, XCBError, XCBResult, XCBWrap};
use crate::check_xcb_error;
use crate::models::{DockArea, Screen, WindowState, WindowType, XyhwChange};
use x11_dl::xlib;
use x11rb::connection::Connection;
use x11rb::properties::{WmClass, WmHints, WmSizeHints, WmSizeHintsSpecification};
use x11rb::protocol::xinerama::{self, is_active, query_screens};
use x11rb::protocol::xproto::{
    self, alloc_named_color, create_colormap, get_atom_name, get_geometry, get_property,
    get_window_attributes, query_pointer, query_tree, Atom, AtomEnum, ColormapAlloc, Window,
};
use x11rb::protocol::Event;

impl XCBWrap {
    // Public functions.

    /// Returns the child windows of all roots.
    /// # Errors
    ///
    /// Will error if root has no windows or there is an error
    /// obtaining the root windows. See `get_windows_for_root`.
    pub fn get_all_windows(&self) -> XCBResult<Vec<Window>> {
        let mut all = Vec::new();
        for root in self.get_roots() {
            match self.get_windows_for_root(root) {
                Ok(some_windows) => {
                    for w in some_windows {
                        all.push(w);
                    }
                }
                Err(err) => return Err(err),
            }
        }
        Ok(all)
    }

    /// Returns a `XColor` for a color.
    // `XDefaultScreen`: https://tronche.com/gui/x/xlib/display/display-macros.html#DefaultScreen
    // `XDefaultColormap`: https://tronche.com/gui/x/xlib/display/display-macros.html#DefaultColormap
    // `XAllocNamedColor`: https://tronche.com/gui/x/xlib/color/XAllocNamedColor.html
    pub fn get_color(&self, color: String) -> XCBResult<u64> {
        let cmap = self.connection.generate_id()?;
        xproto::create_colormap(
            &self.connection,
            ColormapAlloc::NONE,
            cmap,
            self.root,
            self.screen.root_visual,
        )?;
        let color = xproto::alloc_named_color(&self.connection, cmap, color.as_bytes())?.reply()?;
        Ok(color.pixel.into())
    }

    /// Returns the current position of the cursor.
    /// # Errors
    ///
    /// Will error if root window cannot be found.
    // `XQueryPointer`: https://tronche.com/gui/x/xlib/window-information/XQueryPointer.html
    pub fn get_cursor_point(&self) -> XCBResult<(i32, i32)> {
        let roots = self.get_roots();
        for w in roots {
            let reply = xproto::query_pointer(&self.connection, w)?.reply()?;
            if reply.same_screen {
                return Ok((reply.win_x.into(), reply.win_y.into()));
            }
        }
        // TODO: Better error handling for the xcb wrapper
        Err(Box::new(XCBError("Root Window not found")))
    }

    /// Returns the current window under the cursor.
    /// # Errors
    ///
    /// Will error if root window cannot be found.
    // `XQueryPointer`: https://tronche.com/gui/x/xlib/window-information/XQueryPointer.html
    pub fn get_cursor_window(&self) -> XCBResult<WindowHandle> {
        let roots = self.get_roots();
        for w in roots {
            let reply = xproto::query_pointer(&self.connection, w)?.reply()?;
            if reply.same_screen {
                return Ok(WindowHandle::XCBHandle(reply.child));
            }
        }
        // TODO: Better error handling for the xcb wrapper
        Err(Box::new(XCBError("Root Window not found")))
    }

    /// Returns the handle of the default root.
    #[must_use]
    pub const fn get_default_root_handle(&self) -> WindowHandle {
        WindowHandle::XCBHandle(self.root)
    }

    /// Returns the default root.
    #[must_use]
    pub const fn get_default_root(&self) -> xproto::Window {
        self.root
    }

    /// Returns the `WM_SIZE_HINTS`/`WM_NORMAL_HINTS` of a window as a `XyhwChange`.
    #[must_use]
    pub fn get_hint_sizing_as_xyhw(&self, window: xproto::Window) -> XCBResult<XyhwChange> {
        let hint = self.get_hint_sizing(window)?;
        let mut xyhw = XyhwChange::default();

        if let Some(size) = hint.size {
            xyhw.w = Some(size.1);
            xyhw.h = Some(size.2);
        }
        if let Some(size) = hint.size_increment {
            xyhw.w = Some(size.0);
            xyhw.h = Some(size.1);
        }
        if let Some(size) = hint.max_size {
            xyhw.w = Some(size.0);
            xyhw.h = Some(size.1);
        }
        if let Some(size) = hint.min_size {
            xyhw.w = Some(size.0);
            xyhw.h = Some(size.1);
        }
        // Make sure that width and height are not smaller than the min values.
        xyhw.w = std::cmp::max(xyhw.w, xyhw.minw);
        xyhw.h = std::cmp::max(xyhw.h, xyhw.minh);
        // Ignore the sizing if the sizing is set to 0.
        xyhw.w = xyhw.w.filter(|&w| w != 0);
        xyhw.h = xyhw.h.filter(|&h| h != 0);

        if let Some(pos) = hint.position {
            xyhw.x = Some(pos.1);
            xyhw.y = Some(pos.2);
        }

        // TODO: support min/max aspect
        // if size.flags & xlib::PAspect != 0 {
        //     //c->mina = (float)size.min_aspect.y / size.min_aspect.x;
        //     //c->maxa = (float)size.max_aspect.x / size.max_aspect.y;
        // }

        Ok(xyhw)

        // if let Some(size) = hint {
        //     let mut xyhw = XyhwChange::default();
        //
        //     if (size.flags & xlib::PSize) != 0 || (size.flags & xlib::USSize) != 0 {
        //         // These are obsolete but are still used sometimes.
        //         xyhw.w = Some(size.width);
        //         xyhw.h = Some(size.height);
        //     } else if (size.flags & xlib::PBaseSize) != 0 {
        //         xyhw.w = Some(size.base_width);
        //         xyhw.h = Some(size.base_height);
        //     }
        //
        //     if (size.flags & xlib::PResizeInc) != 0 {
        //         xyhw.w = Some(size.width_inc);
        //         xyhw.h = Some(size.height_inc);
        //     }
        //
        //     if (size.flags & xlib::PMaxSize) != 0 {
        //         xyhw.maxw = Some(size.max_width);
        //         xyhw.maxh = Some(size.max_height);
        //     }
        //
        //     if (size.flags & xlib::PMinSize) != 0 {
        //         xyhw.minw = Some(size.min_width);
        //         xyhw.minh = Some(size.min_height);
        //     }
        //     // Make sure that width and height are not smaller than the min values.
        //     xyhw.w = std::cmp::max(xyhw.w, xyhw.minw);
        //     xyhw.h = std::cmp::max(xyhw.h, xyhw.minh);
        //     // Ignore the sizing if the sizing is set to 0.
        //     xyhw.w = xyhw.w.filter(|&w| w != 0);
        //     xyhw.h = xyhw.h.filter(|&h| h != 0);
        //
        //     if (size.flags & xlib::PPosition) != 0 || (size.flags & xlib::USPosition) != 0 {
        //         // These are obsolete but are still used sometimes.
        //         xyhw.x = Some(size.x);
        //         xyhw.y = Some(size.y);
        //     }
        //     // TODO: support min/max aspect
        //     // if size.flags & xlib::PAspect != 0 {
        //     //     //c->mina = (float)size.min_aspect.y / size.min_aspect.x;
        //     //     //c->maxa = (float)size.max_aspect.x / size.max_aspect.y;
        //     // }
        //
        //     return Some(xyhw);
        // }
        // None
    }

    // /// Returns the next `Xevent` that matches the mask of the xserver.
    // // `XMaskEvent`: https://tronche.com/gui/x/xlib/event-handling/manipulating-event-queue/XMaskEvent.html
    // // TODO: Find a way to make this work with xcb
    // pub fn get_mask_event(&self) -> XCBResult<Event> {
    //     self.get_next_event()
    //     // unsafe {
    //     //     let mut event: xlib::XEvent = std::mem::zeroed();
    //     //     (self.xlib.XMaskEvent)(
    //     //         self.display,
    //     //         MOUSEMASK | xlib::SubstructureRedirectMask | xlib::ExposureMask,
    //     //         &mut event,
    //     //     );
    //     //     event
    //     // }
    // }

    /// Returns the next `Xevent` of the xserver if there is one.
    #[must_use]
    pub fn get_next_event(&self) -> XCBResult<Option<Event>> {
        Ok(self.connection.poll_for_event()?)
    }

    /// Returns all the screens of the display.
    /// # Panics
    ///
    /// Panics if xorg cannot be contacted (xlib missing, not started, etc.)
    /// Also panics if window attrs cannot be obtained.
    #[must_use]
    pub fn get_screens(&self) -> XCBResult<Vec<Screen>> {
        let xinerama = xinerama::is_active(&self.connection)?.reply()?;
        // Assuming state 0 is inactive
        match xinerama.state {
            0 => {
                // NON-XINERAMA
                // TODO: Update this when get_window_attrs is fixed
                // let roots: Result<Vec<xproto::GetWindowAttributesReply>> = self
                // let roots = self
                //     .get_roots()
                //     .iter()
                //     .map(|w| self.get_window_attrs(*w))
                //     .collect::<XCBResult<Vec<xproto::GetWindowAttributesReply>>>()?;
                let roots = self
                    .get_roots()
                    .iter()
                    .map(|w| {
                        (
                            check_xcb_error!(
                                self.get_window_geometry(*w),
                                (XyhwChange::default(), *w)
                            ),
                            *w,
                        )
                    })
                    .collect::<Vec<(XyhwChange, xproto::Window)>>();
                if roots.len() == 0 {
                    return Err(Box::new(XCBError("Error: No screen were detected")));
                }
                Ok(roots.iter().map(Screen::from).collect())
            }
            _ => {
                let root = self.get_default_root_handle();
                let info = xinerama::query_screens(&self.connection)?.reply()?;
                Ok(info
                    .screen_info
                    .iter()
                    .map(|i| {
                        let mut s = Screen::from(i);
                        s.root = root;
                        s
                    })
                    .collect())
            }
        }
    }

    /// Returns the dimensions of the screens.
    #[must_use]
    pub fn get_screens_area_dimensions(&self) -> XCBResult<(i32, i32)> {
        let mut height = 0;
        let mut width = 0;
        for s in self.get_screens()? {
            height = std::cmp::max(height, s.bbox.height + s.bbox.y);
            width = std::cmp::max(width, s.bbox.width + s.bbox.x);
        }
        Ok((height, width))
    }

    /// Returns the transient parent of a window.
    // `XGetTransientForHint`: https://tronche.com/gui/x/xlib/ICC/client-to-window-manager/XGetTransientForHint.html
    #[must_use]
    pub fn get_transient_for(&self, window: xproto::Window) -> XCBResult<Option<xproto::Window>> {
        let prop = xproto::get_property(
            &self.connection,
            false,
            window,
            xproto::AtomEnum::WM_TRANSIENT_FOR,
            xproto::AtomEnum::WINDOW,
            0,
            0,
        )?
        .reply()?;
        if let Some(parents) = prop.value32() {
            return Ok(Some(parents.collect::<Vec<u32>>()[0]));
        }
        Ok(None)
        // unsafe {
        //     let mut transient: xlib::Window = std::mem::zeroed();
        //     let status: c_int =
        //         (self.xlib.XGetTransientForHint)(self.display, window, &mut transient);
        //     if status > 0 {
        //         Some(transient)
        //     } else {
        //         None
        //     }
        // }
    }

    /// Returns the atom actions of a window.
    // `XGetWindowProperty`: https://tronche.com/gui/x/xlib/window-information/XGetWindowProperty.html
    #[must_use]
    pub fn get_window_actions_atoms(&self, window: xproto::Window) -> XCBResult<Vec<xproto::Atom>> {
        Ok(xproto::get_property(
            &self.connection,
            false,
            window,
            self.atoms._NET_WM_ACTION,
            xproto::AtomEnum::ATOM,
            0,
            0,
        )?
        .reply()?
        .value32()
        .ok_or(Box::new(XCBError("Error parsing atom _NET_WM_ACTION")))?
        .collect())

        // let mut format_return: i32 = 0;
        // let mut nitems_return: c_ulong = 0;
        // let mut bytes_remaining: c_ulong = 0;
        // let mut type_return: xlib::Atom = 0;
        // let mut prop_return: *mut c_uchar = unsafe { std::mem::zeroed() };
        // unsafe {
        //     let status = (self.xlib.XGetWindowProperty)(
        //         self.display,
        //         window,
        //         self.atoms.NetWMAction,
        //         0,
        //         MAX_PROPERTY_VALUE_LEN / 4,
        //         xlib::False,
        //         xlib::XA_ATOM,
        //         &mut type_return,
        //         &mut format_return,
        //         &mut nitems_return,
        //         &mut bytes_remaining,
        //         &mut prop_return,
        //     );
        //     if status == i32::from(xlib::Success) && !prop_return.is_null() {
        //         #[allow(clippy::cast_lossless, clippy::cast_ptr_alignment)]
        //         let ptr = prop_return as *const c_ulong;
        //         let results: &[xlib::Atom] = slice::from_raw_parts(ptr, nitems_return as usize);
        //         return results.to_vec();
        //     }
        //     vec![]
        // }
    }

    /// Returns the attributes of a window.
    /// # Errors
    ///
    /// Will error if window status is 0 (no attributes).
    // `XGetWindowAttributes`: https://tronche.com/gui/x/xlib/window-information/XGetWindowAttributes.html
    pub fn get_window_attrs(
        &self,
        window: xproto::Window,
    ) -> XCBResult<xproto::GetWindowAttributesReply> {
        Ok(xproto::get_window_attributes(&self.connection, window)?.reply()?)
    }

    /// Returns a windows class `WM_CLASS`
    // `XGetClassHint`: https://tronche.com/gui/x/xlib/ICC/client-to-window-manager/XGetClassHint.html
    #[must_use]
    pub fn get_window_class(&self, window: xproto::Window) -> XCBResult<WmClass> {
        Ok(WmClass::get(&self.connection, window)?.reply()?)
        // unsafe {
        //     let mut class_return: xlib::XClassHint = std::mem::zeroed();
        //     let status = (self.xlib.XGetClassHint)(self.display, window, &mut class_return);
        //     if status == 0 {
        //         return None;
        //     }
        //     let res_name =
        //         match CString::from_raw(class_return.res_name.cast::<c_char>()).into_string() {
        //             Ok(s) => s,
        //             Err(_) => return None,
        //         };
        //     let res_class =
        //         match CString::from_raw(class_return.res_class.cast::<c_char>()).into_string() {
        //             Ok(s) => s,
        //             Err(_) => return None,
        //         };
        //     Some((res_name, res_class))
        // }
    }

    /// Returns the geometry of a window as a `XyhwChange` struct.
    /// # Errors
    ///
    /// Errors if Xlib returns a status of 0.
    // `XGetGeometry`: https://tronche.com/gui/x/xlib/window-information/XGetGeometry.html
    pub fn get_window_geometry(&self, window: xproto::Window) -> XCBResult<XyhwChange> {
        let g = xproto::get_geometry(&self.connection, window)?.reply()?;
        Ok(XyhwChange {
            x: Some(g.x.into()),
            y: Some(g.y.into()),
            w: Some(g.width.into()),
            h: Some(g.height.into()),
            ..XyhwChange::default() // TODO: get max/min width/height
        })

        // let mut root_return: xlib::Window = 0;
        // let mut x_return: c_int = 0;
        // let mut y_return: c_int = 0;
        // let mut width_return: c_uint = 0;
        // let mut height_return: c_uint = 0;
        // let mut border_width_return: c_uint = 0;
        // let mut depth_return: c_uint = 0;
        // unsafe {
        //     let status = (self.xlib.XGetGeometry)(
        //         self.display,
        //         window,
        //         &mut root_return,
        //         &mut x_return,
        //         &mut y_return,
        //         &mut width_return,
        //         &mut height_return,
        //         &mut border_width_return,
        //         &mut depth_return,
        //     );
        //     if status == 0 {
        //         return Err(XlibError::FailedStatus);
        //     }
        // }
        // Ok(XyhwChange {
        //     x: Some(x_return),
        //     y: Some(y_return),
        //     w: Some(width_return as i32),
        //     h: Some(height_return as i32),
        //     ..XyhwChange::default()
        // })
    }

    /// Returns a windows name.
    #[must_use]
    pub fn get_window_name(&self, window: xproto::Window) -> XCBResult<String> {
        let prop = self.get_text_prop(window, self.atoms._NET_WM_NAME);
        if prop.is_ok() {
            return prop;
        }
        self.get_window_legacy_name(window)
    }

    /// Returns a `WM_NAME` (not `_NET`windows name).
    #[must_use]
    pub fn get_window_legacy_name(&self, window: xproto::Window) -> XCBResult<String> {
        self.get_text_prop(window, AtomEnum::WM_NAME.into())
    }

    /// Returns a windows `_NET_WM_PID`.
    #[must_use]
    pub fn get_window_pid(&self, window: xproto::Window) -> XCBResult<u32> {
        Ok(xproto::get_property(
            &self.connection,
            false,
            window,
            self.atoms._NET_WM_PID,
            xproto::AtomEnum::CARDINAL,
            0,
            0,
        )?
        .reply()?
        .value32()
        .ok_or(Box::new(XCBError("Error while parsing atom _NET_WM_STATE")))?
        .collect::<Vec<u32>>()[0])
        // unsafe {
        //     #[allow(clippy::cast_lossless, clippy::cast_ptr_alignment)]
        //     let pid = *prop_return.cast::<u32>();
        //     Some(pid)
        // }
    }

    /// Returns the states of a window.
    #[must_use]
    pub fn get_window_states(&self, window: xproto::Window) -> XCBResult<Vec<WindowState>> {
        Ok(self
            .get_window_states_atoms(window)?
            .iter()
            .map(|a| match a {
                x if x == &self.atoms._NET_WM_STATE_MODAL => WindowState::Modal,
                x if x == &self.atoms._NET_WM_STATE_STICKY => WindowState::Sticky,
                x if x == &self.atoms._NET_WM_STATE_MAXIMIZED_VERT => WindowState::MaximizedVert,
                x if x == &self.atoms._NET_WM_STATE_MAXIMIZED_HORZ => WindowState::MaximizedHorz,
                x if x == &self.atoms._NET_WM_STATE_SHADED => WindowState::Shaded,
                x if x == &self.atoms._NET_WM_STATE_SKIP_TASKBAR => WindowState::SkipTaskbar,
                x if x == &self.atoms._NET_WM_STATE_SKIP_PAGER => WindowState::SkipPager,
                x if x == &self.atoms._NET_WM_STATE_HIDDEN => WindowState::Hidden,
                x if x == &self.atoms._NET_WM_STATE_FULLSCREEN => WindowState::Fullscreen,
                x if x == &self.atoms._NET_WM_STATE_ABOVE => WindowState::Above,
                x if x == &self.atoms._NET_WM_STATE_BELOW => WindowState::Below,
                _ => WindowState::Modal,
            })
            .collect())
    }

    /// Returns the atom states of a window.
    // `XGetWindowProperty`: https://tronche.com/gui/x/xlib/window-information/XGetWindowProperty.html
    #[must_use]
    pub fn get_window_states_atoms(&self, window: Window) -> XCBResult<Vec<Atom>> {
        Ok(xproto::get_property(
            &self.connection,
            false,
            window,
            self.atoms._NET_WM_STATE,
            AtomEnum::ATOM,
            0,
            0,
        )?
        .reply()?
        .value32()
        .ok_or(Box::new(XCBError("Error while parsing atom _NET_WM_STATE")))?
        .collect())
        //
        // let mut format_return: i32 = 0;
        // let mut nitems_return: c_ulong = 0;
        // let mut bytes_remaining: c_ulong = 0;
        // let mut type_return: xlib::Atom = 0;
        // let mut prop_return: *mut c_uchar = unsafe { std::mem::zeroed() };
        // unsafe {
        //     let status = (self.xlib.XGetWindowProperty)(
        //         self.display,
        //         window,
        //         self.atoms.NetWMState,
        //         0,
        //         MAX_PROPERTY_VALUE_LEN / 4,
        //         xlib::False,
        //         xlib::XA_ATOM,
        //         &mut type_return,
        //         &mut format_return,
        //         &mut nitems_return,
        //         &mut bytes_remaining,
        //         &mut prop_return,
        //     );
        //     if status == i32::from(xlib::Success) && !prop_return.is_null() {
        //         #[allow(clippy::cast_lossless, clippy::cast_ptr_alignment)]
        //         let ptr = prop_return as *const c_ulong;
        //         let results: &[xlib::Atom] = slice::from_raw_parts(ptr, nitems_return as usize);
        //         return results.to_vec();
        //     }
        //     vec![]
        // }
    }

    /// Returns structure of a window as a `DockArea`.
    // TODO: Better error handling
    #[must_use]
    pub fn get_window_strut_array(&self, window: xproto::Window) -> XCBResult<Option<DockArea>> {
        // More modern structure.
        if let Some(d) = self.get_window_strut_array_strut_partial(window)? {
            log::debug!("STRUT:[{:?}] {:?}", window, d);
            return Ok(Some(d));
        }
        // Older structure.
        if let Some(d) = self.get_window_strut_array_strut(window)? {
            log::debug!("STRUT:[{:?}] {:?}", window, d);
            return Ok(Some(d));
        }
        Ok(None)
    }

    /// Returns the type of a window.
    #[must_use]
    pub fn get_window_type(&self, window: xproto::Window) -> XCBResult<WindowType> {
        let atom = xproto::get_property(
            &self.connection,
            false,
            window,
            self.atoms._NET_WM_WINDOW_TYPE,
            xproto::AtomEnum::ATOM,
            0,
            0,
        )?
        .reply()?
        .value32()
        .ok_or(Box::new(XCBError("Error parsing atom _NET_WM_ACTION")))?
        .collect::<Vec<u32>>()[0];
        Ok(match atom {
            x if x == self.atoms._NET_WM_WINDOW_TYPE_DESKTOP => WindowType::Desktop,
            x if x == self.atoms._NET_WM_WINDOW_TYPE_DOCK => WindowType::Dock,
            x if x == self.atoms._NET_WM_WINDOW_TYPE_TOOLBAR => WindowType::Toolbar,
            x if x == self.atoms._NET_WM_WINDOW_TYPE_MENU => WindowType::Menu,
            x if x == self.atoms._NET_WM_WINDOW_TYPE_UTILITY => WindowType::Utility,
            x if x == self.atoms._NET_WM_WINDOW_TYPE_SPLASH => WindowType::Splash,
            x if x == self.atoms._NET_WM_WINDOW_TYPE_DIALOG => WindowType::Dialog,
            _ => WindowType::Normal,
        })
    }

    /// Returns the `WM_HINTS` of a window.
    // `XGetWMHints`: https://tronche.com/gui/x/xlib/ICC/client-to-window-manager/XGetWMHints.html
    #[must_use]
    pub fn get_wmhints(&self, window: xproto::Window) -> XCBResult<WmHints> {
        Ok(WmHints::get(&self.connection, window)?.reply()?)
        //
        // unsafe {
        //     let hints_ptr: *const xlib::XWMHints = (self.xlib.XGetWMHints)(self.display, window);
        //     if hints_ptr.is_null() {
        //         return None;
        //     }
        //     let hints: xlib::XWMHints = *hints_ptr;
        //     Some(hints)
        // }
    }

    /// Returns the `WM_STATE` of a window.
    // Really not sure if this works
    // For infos on `WM_STATE`: https://tronche.com/gui/x/xlib/ICC/
    pub fn get_wm_state(&self, window: xproto::Window) -> XCBResult<Option<u32>> {
        let prop = xproto::get_property(
            &self.connection,
            false,
            window,
            self.atoms.WM_STATE,
            self.atoms.WM_STATE,
            0,
            100,
        )?
        .reply()?;
        if let Some(state) = prop.value32() {
            return Ok(Some(state.collect::<Vec<u32>>()[0]));
        }
        Ok(None)
    }

    /// Returns the name of a `XAtom`.
    /// # Errors
    ///
    /// Errors if `XAtom` is not valid.
    // `XGetAtomName`: https://tronche.com/gui/x/xlib/window-information/XGetAtomName.html
    pub fn get_xatom_name(&self, atom: Atom) -> XCBResult<String> {
        Ok(String::from_utf8(
            xproto::get_atom_name(&self.connection, atom)?.reply()?.name,
        )?)
    }

    // Internal functions.

    /// Returns the `WM_SIZE_HINTS`/`WM_NORMAL_HINTS` of a window.
    // `XGetWMNormalHints`: https://tronche.com/gui/x/xlib/ICC/client-to-window-manager/XGetWMNormalHints.html
    #[must_use]
    fn get_hint_sizing(&self, window: xproto::Window) -> XCBResult<WmSizeHints> {
        Ok(WmSizeHints::get(&self.connection, window, AtomEnum::WM_NORMAL_HINTS)?.reply()?)
        // let mut xsize: xlib::XSizeHints = unsafe { std::mem::zeroed() };
        // let mut msize: c_long = xlib::PSize;
        // let status =
        //     unsafe { (self.xlib.XGetWMNormalHints)(self.display, window, &mut xsize, &mut msize) };
        // match status {
        //     0 => None,
        //     _ => Some(xsize),
        // }
    }

    /// Returns a cardinal property of a window.
    /// # Errors
    ///
    /// Errors if window status = 0.
    // `XGetWindowProperty`: https://tronche.com/gui/x/xlib/window-information/XGetWindowProperty.html
    fn get_property(
        &self,
        window: xproto::Window,
        property: xproto::Atom,
        r#type: xproto::Atom,
    ) -> XCBResult<Vec<u8>> {
        let mut prop =
            xproto::get_property(&self.connection, false, window, property, r#type, 0, 0)?
                .reply()?;

        // Correct the type if the one provided is not the actual atom type
        if prop.type_ != r#type {
            prop =
                xproto::get_property(&self.connection, false, window, property, prop.type_, 0, 0)?
                    .reply()?
        }
        Ok(prop.value.to_vec())
        // ) -> Result<(*const c_uchar, c_ulong), XlibError> {
        // let mut format_return: i32 = 0;
        // let mut nitems_return: c_ulong = 0;
        // let mut type_return: xlib::Atom = 0;
        // let mut bytes_after_return: xlib::Atom = 0;
        // let mut prop_return: *mut c_uchar = unsafe { std::mem::zeroed() };
        // unsafe {
        //     let status = (self.xlib.XGetWindowProperty)(
        //         self.display,
        //         window,
        //         property,
        //         0,
        //         MAX_PROPERTY_VALUE_LEN / 4,
        //         xlib::False,
        //         r#type,
        //         &mut type_return,
        //         &mut format_return,
        //         &mut nitems_return,
        //         &mut bytes_after_return,
        //         &mut prop_return,
        //     );
        //     if status == i32::from(xlib::Success) && !prop_return.is_null() {
        //         return Ok((prop_return, nitems_return));
        //     }
        // };
        // Err(XlibError::FailedStatus)
    }

    /// Returns all the roots of the display.
    // `XRootWindowOfScreen`: https://tronche.com/gui/x/xlib/display/screen-information.html#RootWindowOfScreen
    #[must_use]
    fn get_roots(&self) -> Vec<xproto::Window> {
        self.connection
            .setup()
            .roots
            .iter()
            .map(|s| s.root)
            .collect()
        // Ok(self.get_xscreens()
        //     .into_iter()
        //     .map(|mut s| get_screen_info(&self.connection, s))
        //     .map(|mut s| unsafe { (self.xlib.XRootWindowOfScreen)(&mut s) })
        //     .collect())
    }

    /// Returns a text property for a window.
    /// # Errors
    ///
    /// Errors if window status = 0.
    // `XGetTextProperty`: https://tronche.com/gui/x/xlib/ICC/client-to-window-manager/XGetTextProperty.html
    // `XTextPropertyToStringList`: https://tronche.com/gui/x/xlib/ICC/client-to-window-manager/XTextPropertyToStringList.html
    // `XmbTextPropertyToTextList`: https://tronche.com/gui/x/xlib/ICC/client-to-window-manager/XmbTextPropertyToTextList.html
    // TODO: Check if it is useful, maybe merge with get property
    fn get_text_prop(&self, window: xproto::Window, atom: xproto::Atom) -> XCBResult<String> {
        Ok(String::from_utf8(self.get_property(
            window,
            atom,
            AtomEnum::STRING.into(),
        )?)?)
        // unsafe {
        //     let mut text_prop: xlib::XTextProperty = std::mem::zeroed();
        //     let status: c_int =
        //         (self.xlib.XGetTextProperty)(self.display, window, &mut text_prop, atom);
        //     if status == 0 {
        //         return Err(XlibError::FailedStatus);
        //     }
        //     if let Ok(s) = CString::from_raw(text_prop.value.cast::<c_char>()).into_string() {
        //         return Ok(s);
        //     }
        // };
        // Err(XlibError::FailedStatus)
    }

    /// Returns the child windows of a root.
    /// # Errors
    ///
    /// Will error if unknown window status is returned.
    // `XQueryTree`: https://tronche.com/gui/x/xlib/window-information/XQueryTree.html
    fn get_windows_for_root(&self, root: xproto::Window) -> XCBResult<Vec<xproto::Window>> {
        // TODO: check if returning a reference of ok
        Ok(xproto::query_tree(&self.connection, root)?
            .reply()?
            .children)
        // unsafe {
        //     let mut root_return: xlib::Window = std::mem::zeroed();
        //     let mut parent_return: xlib::Window = std::mem::zeroed();
        //     let mut array: *mut xlib::Window = std::mem::zeroed();
        //     let mut length: c_uint = std::mem::zeroed();
        //     let status: xlib::Status = (self.xlib.XQueryTree)(
        //         self.display,
        //         root,
        //         &mut root_return,
        //         &mut parent_return,
        //         &mut array,
        //         &mut length,
        //     );
        //     let windows: &[xlib::Window] = slice::from_raw_parts(array, length as usize);
        //     match status {
        //         0 /* XcmsFailure */ => { Err("Could not load list of windows".to_string() ) }
        //         1 /* XcmsSuccess */ | 2 /* XcmsSuccessWithCompression */ => { Ok(windows) }
        //         _ => { Err("Unknown return status".to_string() ) }
        //     }
        // }
    }

    /// Returns the `_NET_WM_STRUT` as a `DockArea`.
    fn get_window_strut_array_strut(&self, window: xproto::Window) -> XCBResult<Option<DockArea>> {
        let prop: Vec<i32> = xproto::get_property(
            &self.connection,
            false,
            window,
            self.atoms._NET_WM_STRUT,
            AtomEnum::CARDINAL,
            0,
            0,
        )?
        .reply()?
        .value32()
        .unwrap()
        .map(|e| unsafe { transmute::<u32, i32>(e) })
        .collect();
        if prop.len() == 12 {
            let a = prop.as_slice();
            return Ok(Some(DockArea::from(a)));
        }
        Ok(None)
        // unsafe {
        //     #[allow(clippy::cast_ptr_alignment)]
        //     let array_ptr = prop_return.cast::<c_long>();
        //     let slice = slice::from_raw_parts(array_ptr, nitems_return as usize);
        //     if slice.len() == 12 {
        //         return Some(DockArea::from(slice));
        //     }
        //     None
        // }
    }

    /// Returns the `_NET_WM_STRUT_PARTIAL` as a `DockArea`.
    fn get_window_strut_array_strut_partial(
        &self,
        window: xproto::Window,
    ) -> XCBResult<Option<DockArea>> {
        let prop: Vec<i32> = xproto::get_property(
            &self.connection,
            false,
            window,
            self.atoms._NET_WM_STRUT_PARTIAL,
            AtomEnum::CARDINAL,
            0,
            0,
        )?
        .reply()?
        .value32()
        .unwrap()
        .map(|e| unsafe { transmute::<u32, i32>(e) })
        .collect();
        if prop.len() == 12 {
            let a = prop.as_slice();
            return Ok(Some(DockArea::from(a)));
        }
        Ok(None)
        // let (prop_return, nitems_return) = self
        //     .get_property(window, self.atoms.NetWMStrutPartial, xlib::XA_CARDINAL)
        //     .ok()?;
        // unsafe {
        //     #[allow(clippy::cast_ptr_alignment)]
        //     let array_ptr = prop_return.cast::<c_long>();
        //     let slice = slice::from_raw_parts(array_ptr, nitems_return as usize);
        //     if slice.len() == 12 {
        //         return Some(DockArea::from(slice));
        //     }
        //     None
        // }
    }

    /// Returns all the xscreens of the display.
    // `XScreenCount`: https://tronche.com/gui/x/xlib/display/display-macros.html#ScreenCount
    // `XScreenOfDisplay`: https://tronche.com/gui/x/xlib/display/display-macros.html#ScreensOfDisplay
    #[must_use]
    fn get_xscreens(&self) -> Vec<xproto::Screen> {
        self.connection.setup().roots.clone()
        // let mut screens = Vec::new();
        // let screen_count = unsafe { (self.xlib.XScreenCount)(self.display) };
        // for screen_num in 0..(screen_count) {
        //     let screen = unsafe { *(self.xlib.XScreenOfDisplay)(self.display, screen_num) };
        //     screens.push(screen);
        // }
        // screens
    }
}
