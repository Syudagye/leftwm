//! Xlib calls related to a keyboard.
use super::{XCBError, XCBResult, XCBWrap};
use crate::{config::Keybind, utils::xkeysym_lookup};
use std::{os::unix::prelude::IntoRawFd, ptr};
use x11_dl::xlib::Xlib;
use x11rb::protocol::xproto::{
    self, grab_key, ungrab_key, GrabMode, Keycode, Keysym, MappingNotifyEvent, ModMask,
};
use xcb_util::keysyms::KeySymbols;

impl XCBWrap {
    /// Grabs the keysym with the modifier for a window.
    // `XKeysymToKeycode`: https://tronche.com/gui/x/xlib/utilities/keyboard/XKeysymToKeycode.html
    // `XGrabKey`: https://tronche.com/gui/x/xlib/input/XGrabKey.html
    pub fn grab_keys(
        &self,
        root: xproto::Window,
        keysym: u32,
        modifiers: xproto::ModMask,
    ) -> XCBResult<()> {
        // Needed for translating keysym into keycode
        // let c = xcb::Connection::connect_to_fd(self.connection.stream().into_raw_fd(), None)?;
        let (c, _) = xcb::Connection::connect(None)?;
        let keys = KeySymbols::new(&c);
        // let code = unsafe { (self.xlib.XKeysymToKeycode)(self.display, c_ulong::from(keysym)) };
        // TODO: use xcb_utils, maybe create my own crate
        // let xlib = Xlib::open()?;
        // let display = unsafe { (xlib.XOpenDisplay)(ptr::null()) };
        // let code = unsafe { (xlib.XKeysymToKeycode)(display, keysym.into()) };
        let code = keys
            .get_keycode(keysym)
            .next()
            .ok_or("Error getting the keycode")?;

        // Grab the keys with and without numlock (Mod2).
        let mods = [
            modifiers,
            ModMask::from(modifiers | u16::from(ModMask::M2)),
            ModMask::from(modifiers | u16::from(ModMask::LOCK)),
        ];
        for m in mods {
            xproto::grab_key(
                &self.connection,
                true,
                root,
                m,
                code,
                GrabMode::ASYNC,
                GrabMode::ASYNC,
            )?;
            // unsafe {
            //     (self.xlib.XGrabKey)(
            //         self.display,
            //         i32::from(code),
            //         *m,
            //         root,
            //         1,
            //         xlib::GrabModeAsync,
            //         xlib::GrabModeAsync,
            //     );
            // }
        }
        Ok(())
    }

    /// Resets the keybindings to a list of keybindings.
    // `XUngrabKey`: https://tronche.com/gui/x/xlib/input/XUngrabKey.html
    pub fn reset_grabs(&self, keybinds: &[Keybind]) -> XCBResult<()> {
        // Cleanup key grabs.
        // Using 0 as the key, seems to be the code for "Any" key
        xproto::ungrab_key(&self.connection, 0, self.root, xproto::ModMask::ANY)?;
        // unsafe {
        //     (self.xlib.XUngrabKey)(self.display, xlib::AnyKey, xlib::AnyModifier, self.root);
        // }

        // Grab all the key combos from the config file.
        for kb in keybinds {
            if let Some(keysym) = xkeysym_lookup::into_keysym_xcb(&kb.key) {
                let modmask = xkeysym_lookup::into_modmask_xcb(&kb.modifier);
                self.grab_keys(self.root, keysym, modmask)?;
            }
        }
        Ok(())
    }

    /// Updates the keyboard mapping.
    /// # Errors
    ///
    /// Will error if updating the keyboard failed.
    // `XRefreshKeyboardMapping`: https://tronche.com/gui/x/xlib/utilities/keyboard/XRefreshKeyboardMapping.html
    pub fn refresh_keyboard(&self, evt: xproto::MappingNotifyEvent) -> XCBResult<()> {
        // let c = xcb::Connection::connect_to_fd(self.connection.stream().into_raw_fd(), None);
        let (c, _) = xcb::Connection::connect(None)?;
        let keys = KeySymbols::new(&c);
        let ev =
            xcb::xproto::MappingNotifyEvent::new(evt.request.into(), evt.first_keycode, evt.count);
        let status = keys.refresh_keyboard_mapping(&ev);
        // TODO: use xcb_utils, maybe create my own crate
        // let xlib = Xlib::open()?;
        // let display = unsafe { (xlib.XOpenDisplay)(ptr::null()) };
        // let status = unsafe { (self.xlib.XRefreshKeyboardMapping)(evt) };
        if status == 0 {
            Err(Box::new(XCBError(
                "An error occured when refreshing keyboard mapping",
            )))
        } else {
            Ok(())
        }
    }

    /// Converts a keycode to a keysym.
    // `XkbKeycodeToKeysym`: https://linux.die.net/man/3/xkbkeycodetokeysym
    #[must_use]
    pub fn keycode_to_keysym(&self, keycode: xproto::Keycode) -> XCBResult<xproto::Keysym> {
        // let c = xcb::Connection::connect_to_fd(self.connection.stream().into_raw_fd(), None)?;
        let (c, _) = xcb::Connection::connect(None)?;
        let keys = KeySymbols::new(&c);
        Ok(keys.get_keysym(keycode, 0))
        // Not using XKeysymToKeycode because deprecated.
        // let sym = unsafe { (self.xlib.XkbKeycodeToKeysym)(self.display, keycode as u8, 0, 0) };
        // sym as u32
    }

    /// Converts a keysym to a keycode.
    // `XKeysymToKeycode`: https://tronche.com/gui/x/xlib/utilities/keyboard/XKeysymToKeycode.html
    pub fn keysym_to_keycode(&self, keysym: xproto::Keysym) -> XCBResult<xproto::Keycode> {
        let (c, _) = xcb::Connection::connect(None)?;
        let keys = KeySymbols::new(&c);
        keys.get_keycode(keysym)
            .next()
            .ok_or(Box::new(XCBError("Unable to find the keycode")))
        // let code = unsafe { (self.xlib.XKeysymToKeycode)(self.display, keysym.into()) };
        // u32::from(code)
    }
}
