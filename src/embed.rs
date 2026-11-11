//! A plain X11 child window inside the main window. mpv draws into it, and we
//! move/resize it to match the TV screen, and hide it during static.

use x11rb::connection::Connection;
use x11rb::protocol::xproto::{ConfigureWindowAux, ConnectionExt, CreateWindowAux, WindowClass};
use x11rb::rust_connection::RustConnection;

pub struct VideoWindow {
    conn: RustConnection,
    pub id: u32,
    visible: bool,
    geom: (i32, i32, u32, u32),
}

impl VideoWindow {
    pub fn new(parent: u32) -> Result<Self, String> {
        let (conn, screen) = x11rb::connect(None).map_err(|e| e.to_string())?;
        let black = conn.setup().roots[screen].black_pixel;
        let id = conn.generate_id().map_err(|e| e.to_string())?;
        conn.create_window(
            x11rb::COPY_DEPTH_FROM_PARENT,
            id,
            parent,
            0,
            0,
            16,
            16,
            0,
            WindowClass::INPUT_OUTPUT,
            x11rb::COPY_FROM_PARENT,
            &CreateWindowAux::new().background_pixel(black),
        )
        .map_err(|e| e.to_string())?;
        conn.flush().map_err(|e| e.to_string())?;
        Ok(Self { conn, id, visible: false, geom: (0, 0, 16, 16) })
    }

    pub fn set_geometry(&mut self, x: i32, y: i32, w: u32, h: u32) {
        let g = (x, y, w.max(1), h.max(1));
        if g != self.geom {
            self.geom = g;
            let aux = ConfigureWindowAux::new().x(g.0).y(g.1).width(g.2).height(g.3);
            let _ = self.conn.configure_window(self.id, &aux);
            let _ = self.conn.flush();
        }
    }

    pub fn set_visible(&mut self, v: bool) {
        if v != self.visible {
            self.visible = v;
            let _ = if v { self.conn.map_window(self.id).map(|_| ()) } else { self.conn.unmap_window(self.id).map(|_| ()) };
            let _ = self.conn.flush();
        }
    }
}
