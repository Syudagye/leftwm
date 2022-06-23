use crate::{
    models::{Screen, TagId, WindowHandle, WindowState},
    Config, DisplayAction, DisplayEvent, DisplayServer, Keybind, Mode, Window,
};
use signal_hook::low_level::exit;
use x11rb::{
    protocol::{
        xinerama::{self, query_screens},
        xproto::{self, ConnectionExt},
    },
    rust_connection::RustConnection,
};

use self::xcbwrap::{XCBWrap, ICONIC_STATE};

mod event_translate;
mod event_translate_client_message;
mod event_translate_property_notify;
mod xatom;
mod xcbwrap;

use event_translate::XCBEvent;

/// Macro to handle error which could originate from call to XCB
///
/// On error, returns the given expression for function it's called in.
///
/// # Parameters
///
/// The first parameter ($call), is the method/function call that returns a XCBResult<T>.
/// The second parameter ($rt), is what should be returned if there is an error, the default is
/// the unit type `()`
#[macro_export]
macro_rules! check_xcb_error {
    ($call:expr, $rt:expr) => {
        match $call {
            Err(e) => {
                log::error!("Error from XCB: {}", e);
                return $rt;
            }
            Ok(val) => val,
        }
    };

    ($call:expr) => {
        check_xcb_error!($call, ())
    };
}

pub struct XCBDisplayServer {
    xcbwrap: XCBWrap,
}

impl DisplayServer for XCBDisplayServer {
    fn new(config: &impl crate::Config) -> Self {
        let mut xcbw = match XCBWrap::new() {
            Ok(xcbw) => xcbw,
            Err(e) => {
                log::error!("An error occured when connecting to the X server: {}", e);
                exit(1);
            }
        };

        let mut instance = Self { xcbwrap: xcbw };

        check_xcb_error!(instance.xcbwrap.init(config), instance); //setup events masks
        instance
        // let root = wrap.get_default_root();
        // let instance = Self {
        //     xw: wrap,
        //     root,
        //     initial_events: None,
        // };
        // let initial_events = instance.initial_events(config);

        // Self {
        //     initial_events: Some(initial_events),
        //     ..instance
        // }
    }

    fn get_next_events(&mut self) -> Vec<crate::DisplayEvent> {
        let mut events = vec![];

        // if let Some(initial_events) = self.initial_events.take() {
        //     for e in initial_events {
        //         events.push(e);
        //     }
        // }

        loop {
            let xcb_event = match check_xcb_error!(self.xcbwrap.get_next_event(), vec![]) {
                Some(ev) => ev,
                None => break,
            };
            let ev = XCBEvent {
                xcbw: &mut self.xcbwrap,
                event: xcb_event.clone(),
            }
            .into();
            if let Some(e) = ev {
                log::trace!("DisplayEvent: {:?}", xcb_event);
                events.push(e);
            }
        }

        for event in &events {
            if let DisplayEvent::WindowDestroy(WindowHandle::XCBHandle(w)) = event {
                self.xcbwrap.force_unmapped(*w);
            }
        }

        events
    }

    fn load_config(
        &mut self,
        config: &impl crate::Config,
        focused: Option<&Option<crate::models::WindowHandle>>,
        windows: &[crate::Window],
    ) {
        self.xcbwrap.load_config(config, focused, windows);
    }

    fn update_windows(&self, windows: Vec<&crate::Window>) {
        for window in &windows {
            check_xcb_error!(self.xcbwrap.update_window(window));
        }
    }

    fn update_workspaces(&self, focused: Option<&crate::Workspace>) {
        if let Some(focused) = focused {
            check_xcb_error!(self.xcbwrap.set_current_desktop(&focused.tags));
        }
    }

    fn execute_action(&mut self, act: crate::DisplayAction) -> Option<crate::DisplayEvent> {
        log::trace!("DisplayAction: {:?}", act);
        let xcbw = &mut self.xcbwrap;
        let event: Option<DisplayEvent> = match act {
            DisplayAction::KillWindow(h) => from_kill_window(xcbw, h),
            DisplayAction::AddedWindow(h, f, fm) => from_added_window(xcbw, h, f, fm),
            DisplayAction::MoveMouseOver(h, f) => from_move_mouse_over(xcbw, h, f),
            DisplayAction::MoveMouseOverPoint(p) => from_move_mouse_over_point(xcbw, p),
            DisplayAction::DestroyedWindow(h) => from_destroyed_window(xcbw, h),
            DisplayAction::Unfocus(h, f) => from_unfocus(xcbw, h, f),
            DisplayAction::ReplayClick(h, b) => from_replay_click(xcbw, h, b),
            DisplayAction::SetState(h, t, s) => from_set_state(xcbw, h, t, s),
            DisplayAction::SetWindowOrder(fs, ws) => from_set_window_order(xcbw, fs, ws),
            DisplayAction::MoveToTop(h) => from_move_to_top(xcbw, h),
            DisplayAction::ReadyToMoveWindow(h) => from_ready_to_move_window(xcbw, h),
            DisplayAction::ReadyToResizeWindow(h) => from_ready_to_resize_window(xcbw, h),
            DisplayAction::SetCurrentTags(ts) => from_set_current_tags(xcbw, &ts),
            DisplayAction::SetWindowTags(h, ts) => from_set_window_tags(xcbw, h, &ts),
            DisplayAction::ReloadKeyGrabs(ks) => from_reload_key_grabs(xcbw, &ks),
            DisplayAction::ConfigureXlibWindow(w) => from_configure_xlib_window(xcbw, &w),

            DisplayAction::WindowTakeFocus {
                window,
                previous_window,
            } => from_window_take_focus(xcbw, &window, &previous_window),

            DisplayAction::FocusWindowUnderCursor => from_focus_window_under_cursor(xcbw),
            DisplayAction::NormalMode => from_normal_mode(xcbw),
        };
        if event.is_some() {
            log::trace!("DisplayEvent: {:?}", event);
        }
        event
    }

    fn wait_readable(&self) -> std::pin::Pin<Box<dyn futures::Future<Output = ()>>> {
        let task_notify = self.xcbwrap.task_notify.clone();
        Box::pin(async move {
            task_notify.notified().await;
        })
    }

    fn flush(&self) {
        check_xcb_error!(self.xcbwrap.flush());
    }

    fn generate_verify_focus_event(&self) -> Option<crate::DisplayEvent> {
        let handle = check_xcb_error!(self.xcbwrap.get_cursor_window(), None);
        Some(DisplayEvent::VerifyFocusedAt(handle))
    }
}

impl XCBDisplayServer {
    /// Return a vec of events for setting up state of WM.
    fn initial_events(&self, config: &impl Config) -> Vec<DisplayEvent> {
        let mut events = vec![];
        if let Some(workspaces) = config.workspaces() {
            if workspaces.is_empty() {
                // tell manager about existing screens
                // TODO: Check if xinerama is active
                // let screens = query_screens(&self.connection).unwrap().reply().unwrap();
                let screens = check_xcb_error!(
                    check_xcb_error!(xinerama::query_screens(&self.xcbwrap.connection), vec![])
                        .reply(),
                    vec![]
                );
                screens
                    .screen_info
                    .iter()
                    .map(|i| {
                        let mut s = Screen::from(i);
                        s.root = self.xcbwrap.get_default_root_handle();
                        s
                    })
                    .for_each(|screen| {
                        let e = DisplayEvent::ScreenCreate(screen);
                        events.push(e);
                    });
            } else {
                for wsc in &workspaces {
                    let mut screen = Screen::from(wsc);
                    screen.root = self.xcbwrap.get_default_root_handle();
                    let e = DisplayEvent::ScreenCreate(screen);
                    events.push(e);
                }
            }
        }

        // Tell manager about existing windows.
        events.append(&mut self.find_all_windows());

        events
    }

    fn find_all_windows(&self) -> Vec<DisplayEvent> {
        let mut all: Vec<DisplayEvent> = Vec::new();
        let handles = check_xcb_error!(self.xcbwrap.get_all_windows(), vec![]);
        handles.iter().for_each(|h| {
            let attrs = check_xcb_error!(self.xcbwrap.get_window_attrs(*h));
            let state = match check_xcb_error!(self.xcbwrap.get_wm_state(*h)) {
                Some(state) => state,
                None => return,
            };
            if attrs.map_state == xproto::MapState::VIEWABLE || state == ICONIC_STATE as u32 {
                if let Some(event) = check_xcb_error!(self.xcbwrap.setup_window(*h)) {
                    all.push(event);
                }
            }
        });
        all
    }
}

// Display actions.
fn from_kill_window(xcbw: &mut XCBWrap, handle: WindowHandle) -> Option<DisplayEvent> {
    check_xcb_error!(xcbw.kill_window(&handle), None);
    None
}

fn from_added_window(
    xcbw: &mut XCBWrap,
    handle: WindowHandle,
    floating: bool,
    follow_mouse: bool,
) -> Option<DisplayEvent> {
    check_xcb_error!(
        xcbw.setup_managed_window(handle, floating, follow_mouse),
        None
    )
}

fn from_move_mouse_over(
    xcbw: &mut XCBWrap,
    handle: WindowHandle,
    force: bool,
) -> Option<DisplayEvent> {
    let window = handle.xcb_handle()?;
    match xcbw.get_cursor_window() {
        Ok(WindowHandle::XCBHandle(cursor_window)) if force || cursor_window != window => {
            let _ = check_xcb_error!(xcbw.move_cursor_to_window(window), None);
        }
        _ => {}
    }
    None
}

fn from_move_mouse_over_point(xcbw: &mut XCBWrap, point: (i32, i32)) -> Option<DisplayEvent> {
    let _ = check_xcb_error!(xcbw.move_cursor_to_point(point), None);
    None
}

fn from_destroyed_window(xcbw: &mut XCBWrap, handle: WindowHandle) -> Option<DisplayEvent> {
    check_xcb_error!(xcbw.teardown_managed_window(&handle, true), None);
    None
}

fn from_unfocus(
    xcbw: &mut XCBWrap,
    handle: Option<WindowHandle>,
    floating: bool,
) -> Option<DisplayEvent> {
    check_xcb_error!(xcbw.unfocus(handle, floating), None);
    None
}

fn from_replay_click(
    xcbw: &mut XCBWrap,
    handle: WindowHandle,
    button: u32,
) -> Option<DisplayEvent> {
    if let WindowHandle::XCBHandle(handle) = handle {
        check_xcb_error!(xcbw.replay_click(handle, button as u8), None);
    }
    None
}

fn from_set_state(
    xcbw: &mut XCBWrap,
    handle: WindowHandle,
    toggle_to: bool,
    window_state: WindowState,
) -> Option<DisplayEvent> {
    // TODO: impl from for windowstate and xlib::Atom
    let state = match window_state {
        WindowState::Modal => xcbw.atoms._NET_WM_STATE_MODAL,
        WindowState::Sticky => xcbw.atoms._NET_WM_STATE_STICKY,
        WindowState::MaximizedVert => xcbw.atoms._NET_WM_STATE_MAXIMIZED_VERT,
        WindowState::MaximizedHorz => xcbw.atoms._NET_WM_STATE_MAXIMIZED_HORZ,
        WindowState::Shaded => xcbw.atoms._NET_WM_STATE_SHADED,
        WindowState::SkipTaskbar => xcbw.atoms._NET_WM_STATE_SKIP_TASKBAR,
        WindowState::SkipPager => xcbw.atoms._NET_WM_STATE_SKIP_PAGER,
        WindowState::Hidden => xcbw.atoms._NET_WM_STATE_HIDDEN,
        WindowState::Fullscreen => xcbw.atoms._NET_WM_STATE_FULLSCREEN,
        WindowState::Above => xcbw.atoms._NET_WM_STATE_ABOVE,
        WindowState::Below => xcbw.atoms._NET_WM_STATE_BELOW,
    };
    check_xcb_error!(xcbw.set_state(handle, toggle_to, state), None);
    None
}

fn from_set_window_order(
    xcbw: &mut XCBWrap,
    fullscreen: Vec<WindowHandle>,
    windows: Vec<WindowHandle>,
) -> Option<DisplayEvent> {
    // Unmanaged windows.
    let unmanaged: Vec<WindowHandle> = xcbw
        .get_all_windows()
        .unwrap_or_default()
        .iter()
        .filter(|&w| *w != xcbw.get_default_root())
        .map(|&w| w.into())
        .filter(|&h| !windows.iter().any(|&w| w == h) || !fullscreen.iter().any(|&w| w == h))
        .collect();
    let all: Vec<WindowHandle> = [fullscreen, unmanaged, windows].concat();
    check_xcb_error!(xcbw.restack(all), None);
    None
}

fn from_move_to_top(xcbw: &mut XCBWrap, handle: WindowHandle) -> Option<DisplayEvent> {
    check_xcb_error!(xcbw.move_to_top(&handle), None);
    None
}

fn from_ready_to_move_window(xcbw: &mut XCBWrap, handle: WindowHandle) -> Option<DisplayEvent> {
    xcbw.set_mode(Mode::ReadyToMove(handle));
    None
}

fn from_ready_to_resize_window(xcbw: &mut XCBWrap, handle: WindowHandle) -> Option<DisplayEvent> {
    xcbw.set_mode(Mode::ReadyToResize(handle));
    None
}

fn from_set_current_tags(xcbw: &mut XCBWrap, tags: &[TagId]) -> Option<DisplayEvent> {
    check_xcb_error!(xcbw.set_current_desktop(tags), None);
    None
}

fn from_set_window_tags(
    xcbw: &mut XCBWrap,
    handle: WindowHandle,
    tags: &[TagId],
) -> Option<DisplayEvent> {
    let window = handle.xcb_handle()?;
    check_xcb_error!(xcbw.set_window_desktop(window, tags), None);
    None
}

fn from_reload_key_grabs(xcbw: &mut XCBWrap, keybinds: &[Keybind]) -> Option<DisplayEvent> {
    check_xcb_error!(xcbw.reset_grabs(keybinds), None);
    None
}

fn from_configure_xlib_window(xcbw: &mut XCBWrap, window: &Window) -> Option<DisplayEvent> {
    check_xcb_error!(xcbw.configure_window(window), None);
    None
}

fn from_window_take_focus(
    xcbw: &mut XCBWrap,
    window: &Window,
    previous_window: &Option<Window>,
) -> Option<DisplayEvent> {
    check_xcb_error!(
        xcbw.window_take_focus(window, previous_window.as_ref()),
        None
    );
    None
}

fn from_focus_window_under_cursor(xcbw: &mut XCBWrap) -> Option<DisplayEvent> {
    let point = xcbw.get_cursor_point().ok()?;
    let evt = DisplayEvent::MoveFocusTo(point.0, point.1);
    // We check if we found a window, if not we just return evt
    let mut handle = check_xcb_error!(xcbw.get_cursor_window(), Some(evt));
    if handle == WindowHandle::XCBHandle(0) {
        handle = xcbw.get_default_root_handle();
    }
    Some(DisplayEvent::WindowTakeFocus(handle))
}

fn from_normal_mode(xcbw: &mut XCBWrap) -> Option<DisplayEvent> {
    xcbw.set_mode(Mode::Normal);
    None
}
