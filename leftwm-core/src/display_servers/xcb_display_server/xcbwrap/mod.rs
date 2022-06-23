use std::error::Error;
use std::fmt::Display;
use std::{
    ffi::CString, intrinsics::transmute, os::unix::prelude::IntoRawFd, sync::Arc, time::Duration,
};

use tokio::sync::{oneshot, Notify};
use x11rb::protocol::xproto::{AtomEnum, self};
use x11rb::resource_manager;
use x11rb::wrapper::ConnectionExt;
use x11rb::{
    atom_manager,
    connection::Connection,
    cursor,
    protocol::{
        randr::{get_crtc_info, get_screen_resources},
        xproto::{
            alloc_named_color, change_property, change_window_attributes, create_colormap,
            delete_property, grab_key, send_event, ungrab_key, Atom, ChangeWindowAttributesAux,
            ClientMessageData, ClientMessageEvent, Colormap, ColormapAlloc, EventMask, GrabMode,
            Keycode, Keysym, ModMask, PropMode, Screen,
        },
    },
    resource_manager::Database,
    rust_connection::RustConnection,
    CURRENT_TIME,
};

use crate::check_xcb_error;
use crate::{
    config::FocusBehaviour,
    models::WindowHandle,
    utils::{self, xkeysym_lookup::into_keysym},
    Config, Keybind, Mode, Window,
};

use super::xatom::AtomCollection;

mod getters;
mod keyboard;
mod mouse;
mod setters;
mod window;

pub type XCBResult<T> = core::result::Result<T, Box<dyn std::error::Error>>;

pub const WITHDRAWN_STATE: u8 = 0;
pub const NORMAL_STATE: u8 = 1;
pub const ICONIC_STATE: u8 = 2;

// Cannot use constants/statics here, because EventMask needs a function call to process the OR operation
// pub const ROOT_EVENT_MASK: EventMask = EventMask::SUBSTRUCTURE_REDIRECT
//     | EventMask::SUBSTRUCTURE_NOTIFY
//     | EventMask::BUTTON_PRESS
//     | EventMask::POINTER_MOTION
//     | EventMask::STRUCTURE_NOTIFY;
//
// const BUTTONMASK: EventMask = EventMask::BUTTON_PRESS
//     | EventMask::BUTTON_MOTION
//     | EventMask::BUTTON_MOTION;
// const MOUSEMASK: EventMask = BUTTONMASK | EventMask::POINTER_MOTION;
#[macro_export]
macro_rules! ROOT_EVENT_MASK {
    () => {
        EventMask::SUBSTRUCTURE_REDIRECT
        | EventMask::SUBSTRUCTURE_NOTIFY
        | EventMask::BUTTON_PRESS
        | EventMask::POINTER_MOTION
        | EventMask::STRUCTURE_NOTIFY
    };
}
#[macro_export]
macro_rules! BUTTONMASK {
    () => {
        EventMask::BUTTON_PRESS
        | EventMask::BUTTON_MOTION
        | EventMask::BUTTON_MOTION
    };
}
#[macro_export]
macro_rules! MOUSEMASK {
    () => {
        BUTTONMASK!()
        | EventMask::POINTER_MOTION
    };
}

/// Colors for the window borders
pub struct Colors {
    normal: u64,
    floating: u64,
    active: u64,
}

/// Attempt to Error Handling
#[derive(Debug, Clone)]
// pub enum XCBError {
//     FailedStatus,
//     RootWindowNotFound,
//     InvalidXAtom,
// }
pub struct XCBError(&'static str);
impl Error for XCBError {}
impl Display for XCBError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self)
    }
}

/// Cursors
#[derive(Debug)]
pub struct XCBCursors {
    pub normal: xproto::Cursor,
    pub resize: xproto::Cursor,
    pub move_: xproto::Cursor,
}
impl XCBCursors {
    pub fn new(connection: &RustConnection, screen: usize) -> XCBResult<Self> {
        let db = Database::new_from_default(connection)?;
        let handle = cursor::Handle::new(connection, screen, &db)?.reply()?;
        Ok(Self {
            normal: handle.load_cursor(connection, "pointer")?,
            resize: handle.load_cursor(connection, "sizing")?,
            move_: handle.load_cursor(connection, "fleur")?,
        })
    }
}

/// Contains Xserver information and origins.
pub struct XCBWrap {
    pub(super) connection: RustConnection,
    screen: Screen,
    root: xproto::Window,
    pub atoms: AtomCollection,
    cursors: XCBCursors,
    colors: Colors,
    pub managed_windows: Vec<xproto::Window>,
    pub focused_window: xproto::Window,
    pub tag_labels: Vec<String>,
    pub mode: Mode,
    pub focus_behaviour: FocusBehaviour,
    pub mouse_key_mask: xproto::ModMask,
    pub mode_origin: (i32, i32),
    _task_guard: oneshot::Receiver<()>,
    pub task_notify: Arc<Notify>,
    pub motion_event_limiter: u64,
    pub refresh_rate: u16,
}

impl XCBWrap {
    // `XOpenDisplay`: https://tronche.com/gui/x/xlib/display/opening.html
    // `XConnectionNumber`: https://tronche.com/gui/x/xlib/display/display-macros.html#ConnectionNumber
    // `XDefaultRootWindow`: https://tronche.com/gui/x/xlib/display/display-macros.html#DefaultRootWindow
    // `XSetErrorHandler`: https://tronche.com/gui/x/xlib/event-handling/protocol-errors/XSetErrorHandler.html
    // `XSelectInput`: https://tronche.com/gui/x/xlib/event-handling/XSelectInput.html
    // TODO: Split this function up.
    pub fn new() -> XCBResult<Self> {
        const SERVER: mio::Token = mio::Token(0);

        // Connecting to the X server
        let (connection, display) = x11rb::connect(None).unwrap();
        let screen = connection.setup().roots[display].clone();

        // Setting up socket (TODO: Improve this comment)
        let (guard, _task_guard) = oneshot::channel();
        let notify = Arc::new(Notify::new());
        let task_notify = notify.clone();

        // let mut poll = mio::Poll::new().expect("Unable to boot Mio");
        let mut events = mio::Events::with_capacity(1);
        // poll.registry()
        //     .register(
        //         &mut mio::unix::SourceFd(&connection.stream())),
        //         SERVER,
        //         mio::Interest::READABLE,
        //     )
        //     .expect("Unable to boot Mio");
        let timeout = Duration::from_millis(100);
        tokio::task::spawn_blocking(move || loop {
            if guard.is_closed() {
                return;
            }

            // if let Err(err) = poll.poll(&mut events, Some(timeout)) {
            //     log::warn!("Xlib socket poll failed with {:?}", err);
            //     continue;
            // }

            events
                .iter()
                .filter(|event| SERVER == event.token())
                .for_each(|_| notify.notify_one());
        });

        let atoms = AtomCollection::new(&connection)?.reply()?;
        let db = Database::new_from_resource_manager(&connection)?.unwrap();
        let cursors = XCBCursors::new(&connection, display)?;
        let colors = Colors {
            normal: 0,
            floating: 0,
            active: 0,
        };

        // Getting refresh rate
        let refresh_rate = {
            let screen_resources = get_screen_resources(&connection, screen.root)?.reply()?;
            let active: Vec<u32> = screen_resources
                .crtcs
                .iter()
                .filter_map(|crtc| {
                    get_crtc_info(&connection, *crtc, screen_resources.config_timestamp).ok()
                })
                .filter_map(|crtc_reply| crtc_reply.reply().ok())
                .filter(|crtc_info| crtc_info.mode != 0)
                .map(|crtc_info| crtc_info.mode)
                .collect();
            screen_resources
                .modes
                .iter()
                .filter(|mode_info| active.contains(&mode_info.id))
                .filter_map(|mode_info| {
                    u16::try_from(
                        mode_info.dot_clock / u32::from(mode_info.htotal * mode_info.vtotal),
                    )
                    .ok()
                })
                .max()
                .unwrap_or(60)
        };
        log::debug!("Refresh rate: {}", refresh_rate);

        // Check that another WM is not running.
        change_window_attributes(
            &connection,
            screen.root,
            &ChangeWindowAttributesAux::new().event_mask(EventMask::SUBSTRUCTURE_REDIRECT),
        )?;
        x11rb::wrapper::ConnectionExt::sync(&connection)?;

        Ok(Self {
            connection,
            screen: screen.clone(),
            root: screen.root,
            atoms,
            cursors,
            colors,
            managed_windows: vec![],
            focused_window: screen.root,
            tag_labels: vec![],
            mode: Mode::Normal,
            focus_behaviour: FocusBehaviour::Sloppy,
            mouse_key_mask: ModMask::ANY,
            mode_origin: (0, 0),
            _task_guard,
            task_notify,
            motion_event_limiter: 0,
            refresh_rate,
        })
    }

    pub fn load_config(
        &mut self,
        config: &impl Config,
        focused: Option<&Option<WindowHandle>>,
        windows: &[Window],
    ) {
        self.focus_behaviour = config.focus_behaviour();
        self.mouse_key_mask = utils::xkeysym_lookup::into_modmask_xcb(&config.mousekey());
        self.load_colors(config, focused, Some(windows));
        self.tag_labels = config.create_list_of_tag_labels();
        check_xcb_error!(self.reset_grabs(&config.mapped_bindings()));
    }

    /// Initialize the xcb wrapper.
    // `XChangeWindowAttributes`: https://tronche.com/gui/x/xlib/window/XChangeWindowAttributes.html
    // `XDeleteProperty`: https://tronche.com/gui/x/xlib/window-information/XDeleteProperty.html
    // TODO: split into smaller functions
    pub fn init(&mut self, config: &impl Config) -> XCBResult<()> {
        self.focus_behaviour = config.focus_behaviour();
        self.mouse_key_mask = utils::xkeysym_lookup::into_modmask_xcb(&config.mousekey());

        let root = self.root;
        self.load_colors(config, None, None);

        let mut attrs = ChangeWindowAttributesAux::new();
        attrs.cursor = Some(self.cursors.normal);
        attrs.event_mask = Some(ROOT_EVENT_MASK!().into());
        change_window_attributes(&self.connection, self.root, &attrs)?;

        self.subscribe_to_event(root, ROOT_EVENT_MASK!().into())?;

        // EWMH compliance.
        let sup: Vec<u8> = self
            .atoms
            .get_net_supported()
            .iter()
            .map(|&atom| atom.to_be_bytes())
            .flatten()
            .collect();
        change_property(
            &self.connection,
            PropMode::REPLACE,
            self.root,
            self.atoms._NET_SUPPORTED,
            AtomEnum::ATOM,
            32,
            sup.len() as u32,
            sup.as_slice(),
        )?;
        delete_property(&self.connection, self.root, self.atoms._NET_CLIENT_LIST)?;
        // EWMH compliance for desktops.
        self.tag_labels = config.create_list_of_tag_labels();
        self.init_desktops_hints()?;

        self.reset_grabs(&config.mapped_bindings())?;

        // self.sync();
        self.connection.sync()?;
        Ok(())
    }

    /// EWMH support used for bars such as polybar.
    // `Xutf8TextListToTextProperty`: https://linux.die.net/man/3/xutf8textlisttotextproperty
    // `XSetTextProperty`: https://tronche.com/gui/x/xlib/ICC/client-to-window-manager/XSetTextProperty.html
    pub fn init_desktops_hints(&self) -> XCBResult<()> {
        let tag_labels = &self.tag_labels;
        let tag_length = tag_labels.len();

        // Set the number of desktop.
        let data = vec![tag_length as u32];
        self.set_desktop_prop(&data, self.atoms._NET_NUMBER_OF_DESKTOPS);

        // Set a current desktop.
        let data = vec![0_u32, CURRENT_TIME];
        self.set_desktop_prop(&data, self.atoms._NET_CURRENT_DESKTOP);

        // TODO: Find a way to implement this
        // Set desktop names.
        // let mut text: xlib::XTextProperty = unsafe { std::mem::zeroed() };
        // unsafe {
        //     let mut clist_tags: Vec<*mut c_char> = tag_labels
        //         .iter()
        //         .map(|x| CString::new(x.clone()).unwrap_or_default().into_raw())
        //         .collect();
        //     let ptr = clist_tags.as_mut_ptr();
        //     (self.xlib.Xutf8TextListToTextProperty)(
        //         self.display,
        //         ptr,
        //         clist_tags.len() as i32,
        //         xlib::XUTF8StringStyle,
        //         &mut text,
        //     );
        //     std::mem::forget(clist_tags);
        //     (self.xlib.XSetTextProperty)(
        //         self.display,
        //         self.root,
        //         &mut text,
        //         self.atoms.NetDesktopNames,
        //     );
        // }

        // Set the WM NAME.
        self.set_desktop_prop_string("LeftWM", self.atoms._NET_WM_NAME, AtomEnum::STRING.into())?;

        self.set_desktop_prop_string("LeftWM", AtomEnum::WM_CLASS.into(), AtomEnum::STRING.into())?;

        self.set_desktop_prop_c_ulong(
            self.root.into(),
            self.atoms._NET_SUPPORTING_WM_CHECK,
            AtomEnum::WINDOW.into(),
        )?;

        // Set a viewport.
        let data = vec![0_u32, 0_u32];
        self.set_desktop_prop(&data, self.atoms._NET_DESKTOP_VIEWPORT)?;

        Ok(())
    }

    /// Send a xevent atom for a window to X.
    // `XSendEvent`: https://tronche.com/gui/x/xlib/event-handling/XSendEvent.html
    fn send_xevent_atom(&self, window: xproto::Window, atom: Atom) -> XCBResult<bool> {
        if self.can_send_xevent_atom(window, atom)? {
            let data = ClientMessageData::from([atom, CURRENT_TIME, 0, 0, 0].to_owned());
            let msg = ClientMessageEvent::new(
                32, window,
                33u32, // looking at the old xlib implementation, xlib::ClientMessage was used, which pointed to the value 33
                data,
            );
            self.send_xevent(window, false, EventMask::NO_EVENT, &msg)?;
            return Ok(true);
        }
        Ok(false)
    }

    /// Send a xevent for a window to X.
    // `XSendEvent`: https://tronche.com/gui/x/xlib/event-handling/XSendEvent.html
    pub fn send_xevent(
        &self,
        window: xproto::Window,
        propagate: bool,
        mask: EventMask,
        event: &ClientMessageEvent,
    ) -> XCBResult<()> {
        send_event(&self.connection, propagate, window, mask, event)?;
        x11rb::wrapper::ConnectionExt::sync(&self.connection)?;
        Ok(())
    }

    /// Returns whether a window can recieve a xevent atom.
    // `XGetWMProtocols`: https://tronche.com/gui/x/xlib/ICC/client-to-window-manager/XGetWMProtocols.html
    fn can_send_xevent_atom(&self, window: xproto::Window, atom: Atom) -> XCBResult<bool> {
        // TODO: find an alternative to XGetWMProtocols
        unimplemented!();
        // unsafe {
        //     let mut array: *mut xlib::Atom = std::mem::zeroed();
        //     let mut length: c_int = std::mem::zeroed();
        //     let status: xlib::Status =
        //         (self.xlib.XGetWMProtocols)(self.display, window, &mut array, &mut length);
        //     let protocols: &[xlib::Atom] = slice::from_raw_parts(array, length as usize);
        //     status > 0 && protocols.contains(&atom)
        // }
    }

    /// Load the colors of our theme.
    pub fn load_colors(
        &mut self,
        config: &impl Config,
        focused: Option<&Option<WindowHandle>>,
        windows: Option<&[Window]>,
    ) {
        self.colors = Colors {
            // TODO: Error handling
            normal: self.get_color(config.default_border_color()).unwrap(),
            floating: self.get_color(config.floating_border_color()).unwrap(),
            active: self.get_color(config.focused_border_color()).unwrap(),
        };
        // Update all the windows with the new colors.
        if let Some(windows) = windows {
            for window in windows {
                if let WindowHandle::XCBHandle(handle) = window.handle {
                    let is_focused =
                        matches!(focused, Some(&Some(focused)) if focused == window.handle);
                    let color = if is_focused {
                        self.colors.active
                    } else if window.floating() {
                        self.colors.floating
                    } else {
                        self.colors.normal
                    };
                    self.set_window_border_color(handle, color);
                }
            }
        }
    }

    /// Sets the mode within our xwrapper.
    pub fn set_mode(&mut self, mode: Mode) {
        match mode {
            // Prevent resizing and moving of root.
            Mode::MovingWindow(h)
            | Mode::ResizingWindow(h)
            | Mode::ReadyToMove(h)
            | Mode::ReadyToResize(h)
                if h == self.get_default_root_handle() => {}
            Mode::ReadyToMove(_) | Mode::ReadyToResize(_) if self.mode == Mode::Normal => {
                self.mode = mode;
                if let Ok(loc) = self.get_cursor_point() {
                    self.mode_origin = loc;
                }
                let cursor = match mode {
                    Mode::ReadyToResize(_) | Mode::ResizingWindow(_) => self.cursors.resize,
                    Mode::ReadyToMove(_) | Mode::MovingWindow(_) => self.cursors.move_,
                    Mode::Normal => self.cursors.normal,
                };
                self.grab_pointer(cursor);
            }
            Mode::MovingWindow(h) | Mode::ResizingWindow(h)
                if self.mode == Mode::ReadyToMove(h) || self.mode == Mode::ReadyToResize(h) =>
            {
                self.ungrab_pointer();
                self.mode = mode;
                let cursor = match mode {
                    Mode::ReadyToResize(_) | Mode::ResizingWindow(_) => self.cursors.resize,
                    Mode::ReadyToMove(_) | Mode::MovingWindow(_) => self.cursors.move_,
                    Mode::Normal => self.cursors.normal,
                };
                self.grab_pointer(cursor);
            }
            Mode::Normal => {
                self.ungrab_pointer();
                self.mode = mode;
            }
            _ => {}
        }
    }

    /// Wait until readable.
    pub async fn wait_readable(&mut self) {
        self.task_notify.notified().await;
    }

    /// Flush and sync the xserver.
    // `XSync`: https://tronche.com/gui/x/xlib/event-handling/XSync.html
    pub fn sync(&self) -> XCBResult<()> {
        Ok(x11rb::wrapper::ConnectionExt::sync(&self.connection)?)
    }

    /// Flush the xserver.
    // `XFlush`: https://tronche.com/gui/x/xlib/event-handling/XFlush.html
    pub fn flush(&self) -> XCBResult<()> {
        Ok(self.connection.flush()?)
    }

    // /// Returns how many events are waiting.
    // // `XPending`: https://tronche.com/gui/x/xlib/event-handling/XPending.html
    // #[must_use]
    // pub fn queue_len(&self) -> i32 {
    //     unsafe { (self.xlib.XPending)(self.display) }
    // }
}
