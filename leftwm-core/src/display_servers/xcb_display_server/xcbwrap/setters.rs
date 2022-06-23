//! `XWrap` setters.
use super::{WindowHandle, XCBResult, XCBWrap};
use crate::models::TagId;
use std::ffi::CString;
use x11_dl::xlib;
use x11rb::properties::WmHints;
use x11rb::protocol::xproto::{
    self, change_property, change_window_attributes, configure_window, delete_property, AtomEnum,
    ChangeWindowAttributesAux, PropMode,
};
use x11rb::wrapper::ConnectionExt;

impl XCBWrap {
    // Public functions.

    /// Appends a window property.
    // `XChangeProperty`: https://tronche.com/gui/x/xlib/window-information/XChangeProperty.html
    pub fn append_property_long(
        &self,
        window: xproto::Window,
        property: xproto::Atom,
        r#type: xproto::Atom,
        data: &[u8],
    ) -> XCBResult<()> {
        xproto::change_property(
            &self.connection,
            PropMode::APPEND,
            window,
            property,
            r#type,
            32,
            data.len().try_into()?,
            data,
        )?;
        Ok(())
    }

    /// Replaces a window property.
    // `XChangeProperty`: https://tronche.com/gui/x/xlib/window-information/XChangeProperty.html
    pub fn replace_property_long(
        &self,
        window: xproto::Window,
        property: xproto::Atom,
        r#type: xproto::Atom,
        data: &[u8],
    ) -> XCBResult<()> {
        change_property(
            &self.connection,
            PropMode::REPLACE,
            window,
            property,
            r#type,
            32,
            data.len().try_into()?,
            data,
        )?;
        Ok(())
    }

    /// Sets the client list to the currently managed windows.
    // `XDeleteProperty`: https://tronche.com/gui/x/xlib/window-information/XDeleteProperty.html
    pub fn set_client_list(&self) -> XCBResult<()> {
        xproto::delete_property(&self.connection, self.root, self.atoms._NET_CLIENT_LIST)?;
        // unsafe {
        //     (self.xlib.XDeleteProperty)(self.display, self.root, self.atoms.NetClientList);
        // }
        for w in &self.managed_windows {
            let list = w.to_be_bytes();
            self.append_property_long(
                self.root,
                self.atoms._NET_CLIENT_LIST,
                AtomEnum::WINDOW.into(),
                &list,
            )?;
        }
        Ok(())
    }

    /// Sets the current desktop.
    pub fn set_current_desktop(&self, current_tags: &[TagId]) -> XCBResult<()> {
        let mut indexes: Vec<u32> = current_tags
            .iter()
            .map(|tag| tag.to_owned() as u32 - 1)
            .collect();
        if indexes.is_empty() {
            indexes.push(0);
        }
        self.set_desktop_prop(&indexes, self.atoms._NET_CURRENT_DESKTOP)
    }

    // /// Sets the current viewport.
    // fn set_current_viewport(&self, tags: Vec<&String>) {
    //     let mut indexes: Vec<u32> = vec![];
    //     for tag in tags {
    //         for (i, mytag) in self.tags.iter().enumerate() {
    //             if tag.contains(mytag) {
    //                 indexes.push(i as u32);
    //             }
    //         }
    //     }
    //     if indexes.is_empty() {
    //         indexes.push(0);
    //     }
    //     self.set_desktop_prop(&indexes, self.atoms.NetDesktopViewport);
    // }

    /// Sets a desktop property.
    pub fn set_desktop_prop(&self, data: &[u32], atom: xproto::Atom) -> XCBResult<()> {
        let x_data: Vec<u8> = data
            .iter()
            .map(|x| x.to_be_bytes().to_vec())
            .fold(vec![], |acc, v| [acc, v].concat());
        self.replace_property_long(self.root, atom, AtomEnum::CARDINAL.into(), &x_data)
    }

    /// Sets a desktop property with type `c_ulong`.
    pub fn set_desktop_prop_c_ulong(
        &self,
        value: u64,
        atom: xproto::Atom,
        r#type: xproto::Atom,
    ) -> XCBResult<()> {
        let data = value.to_be_bytes();
        self.replace_property_long(self.root, atom, r#type, &data)
    }

    /// Sets a desktop property with type string.
    // `XChangeProperty`: https://tronche.com/gui/x/xlib/window-information/XChangeProperty.html
    pub fn set_desktop_prop_string(
        &self,
        value: &str,
        atom: xproto::Atom,
        encoding: xproto::Atom,
    ) -> XCBResult<()> {
        if let Ok(cstring) = CString::new(value) {
            xproto::change_property(
                &self.connection,
                PropMode::REPLACE,
                self.root,
                atom,
                encoding,
                8,
                value.len().try_into()?,
                cstring.as_bytes(),
            )?;
        }
        Ok(())
    }

    /// Sets a windows state.
    pub fn set_state(
        &self,
        handle: WindowHandle,
        toggle_to: bool,
        atom: xproto::Atom,
    ) -> XCBResult<()> {
        if let WindowHandle::XCBHandle(h) = handle {
            let mut states = self.get_window_states_atoms(h)?;
            if toggle_to {
                if states.contains(&atom) {
                    return Ok(());
                }
                states.push(atom);
            } else {
                let index = match states.iter().position(|s| s == &atom) {
                    Some(i) => i,
                    None => return Ok(()),
                };
                states.remove(index);
            }
            self.set_window_states_atoms(h, &states);
        }
        Ok(())
    }

    /// Sets a windows border color.
    // `XSetWindowBorder`: https://tronche.com/gui/x/xlib/window/XSetWindowBorder.html
    pub fn set_window_border_color(&self, window: xproto::Window, mut color: u64) -> XCBResult<()> {
        // TODO: Idk for the colors, need to check which format is better
        let mut bytes = color.to_le_bytes();
        bytes[3] = 0xff;
        color = u64::from_le_bytes(bytes);

        let attrs = ChangeWindowAttributesAux::new().border_pixel(Some(color.try_into()?));
        xproto::change_window_attributes(&self.connection, window, &attrs)?;
        Ok(())
    }

    /// Sets a windows configuration.
    pub fn set_window_config(
        &self,
        window: xproto::Window,
        window_changes: &xproto::ConfigureWindowAux,
    ) -> XCBResult<()> {
        xproto::configure_window(&self.connection, window, window_changes)?;
        self.connection.sync()?;
        Ok(())
    }

    /// Sets what desktop a window is on.
    pub fn set_window_desktop(
        &self,
        window: xproto::Window,
        current_tags: &[TagId],
    ) -> XCBResult<()> {
        let mut indexes: Vec<u8> = current_tags
            .iter()
            .map(|tag| (tag - 1).to_ne_bytes())
            .collect::<Vec<[u8; 8]>>()
            .concat();
        // .fold(vec![], |a, b| [a, b.to_vec()].concat());
        if indexes.is_empty() {
            indexes.push(0);
        }
        self.replace_property_long(
            window,
            self.atoms._NET_WM_DESKTOP,
            xproto::AtomEnum::CARDINAL.into(),
            &indexes,
        )
    }

    /// Sets the atom states of a window.
    pub fn set_window_states_atoms(
        &self,
        window: xproto::Window,
        states: &[xproto::Atom],
    ) -> XCBResult<()> {
        let data: Vec<u8> = states
            .iter()
            .map(|x| (*x).to_be_bytes())
            .collect::<Vec<[u8; 4]>>()
            .concat();
        self.replace_property_long(
            window,
            self.atoms._NET_WM_STATE,
            xproto::AtomEnum::ATOM.into(),
            &data,
        )
    }

    pub fn set_window_urgency(&self, window: xproto::Window, is_urgent: bool) -> XCBResult<()> {
        let mut hints = self.get_wmhints(window)?;
        if hints.urgent == is_urgent {
            return Ok(());
        }
        hints.urgent = is_urgent;
        self.set_wmhints(window, hints)
    }

    /// Sets the `XWMHints` of a window.
    pub fn set_wmhints(&self, window: xproto::Window, wmh: WmHints) -> XCBResult<()> {
        wmh.set(&self.connection, window)?;
        Ok(())
    }

    /// Sets the `WM_STATE` of a window.
    pub fn set_wm_states(&self, window: xproto::Window, states: &[u8]) -> XCBResult<()> {
        self.replace_property_long(window, self.atoms.WM_STATE, self.atoms.WM_STATE, states)?;
        Ok(())
    }
}
