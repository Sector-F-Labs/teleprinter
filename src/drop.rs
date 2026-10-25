//! Files dropped on the window, typed into a pane as shell-quoted paths, as
//! other terminals do (ADR-0014). Claude Code, for one, attaches an image
//! whose path is pasted.
//!
//! egui reports drops on X11, macOS and Windows; winit 0.30 has none on
//! Wayland, so there a data device on winit's own connection takes the drop.

use std::path::PathBuf;

/// Files dropped, and where (in points) if known.
pub struct Dropped {
    pub at: Option<egui::Pos2>,
    pub paths: Vec<PathBuf>,
}

/// Drops that came in since the last frame.
pub struct Drops {
    #[cfg(target_os = "linux")]
    wayland: Option<std::sync::mpsc::Receiver<Dropped>>,
}

impl Drops {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        Self {
            #[cfg(target_os = "linux")]
            wayland: wayland::listen(cc)
                .inspect_err(|e| eprintln!("teleprinter: no file drops: {e}"))
                .ok()
                .flatten(),
        }
    }

    /// Shell-less, for tests.
    #[cfg(test)]
    pub fn none() -> Self {
        Self {
            #[cfg(target_os = "linux")]
            wayland: None,
        }
    }

    pub fn take(&self, ctx: &egui::Context) -> Vec<Dropped> {
        let mut drops = Vec::new();
        ctx.input(|i| {
            let paths: Vec<PathBuf> = i.raw.dropped_files.iter().filter_map(|f| f.path.clone()).collect();
            if !paths.is_empty() {
                drops.push(Dropped { at: i.pointer.latest_pos(), paths });
            }
        });
        #[cfg(target_os = "linux")]
        if let Some(rx) = &self.wayland {
            let zoom = ctx.zoom_factor();
            drops.extend(rx.try_iter().map(|d| Dropped { at: d.at.map(|p| p / zoom), ..d }));
        }
        drops
    }
}

/// The paths as typed: each quoted for the shell if it needs it, separated
/// by spaces, with one after the last to carry on typing.
pub fn typed(paths: &[PathBuf]) -> String {
    paths.iter().map(|p| quote(&p.to_string_lossy()) + " ").collect()
}

fn quote(s: &str) -> String {
    let plain = |c: char| c.is_ascii_alphanumeric() || "/._-+,:@%=".contains(c);
    if !s.is_empty() && s.chars().all(plain) {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', r"'\''"))
    }
}

/// The paths in a `text/uri-list`: `file:` URIs, percent-decoded; anything
/// else is skipped.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn uri_list(list: &str) -> Vec<PathBuf> {
    list.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|l| l.strip_prefix("file://"))
        // A host before the path (`file://host/path`) is skipped.
        .filter_map(|rest| rest.find('/').map(|i| &rest[i..]))
        .map(|path| PathBuf::from(<std::ffi::OsString as std::os::unix::ffi::OsStringExt>::from_vec(percent_decode(path))))
        .collect()
}

fn percent_decode(s: &str) -> Vec<u8> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = |b: u8| (b as char).to_digit(16);
        match (bytes[i], bytes.get(i + 1).copied().and_then(hex), bytes.get(i + 2).copied().and_then(hex)) {
            (b'%', Some(hi), Some(lo)) => {
                out.push((hi * 16 + lo) as u8);
                i += 3;
            }
            (b, ..) => {
                out.push(b);
                i += 1;
            }
        }
    }
    out
}

#[cfg(target_os = "linux")]
mod wayland {
    use std::collections::HashMap;
    use std::io::Read;
    use std::sync::mpsc::{self, Receiver, Sender};

    use raw_window_handle::{HasDisplayHandle, RawDisplayHandle};
    use wayland_backend::client::{Backend, ObjectId};
    use wayland_client::globals::{GlobalListContents, registry_queue_init};
    use wayland_client::protocol::wl_data_device::{self, WlDataDevice};
    use wayland_client::protocol::wl_data_device_manager::{DndAction, WlDataDeviceManager};
    use wayland_client::protocol::wl_data_offer::{self, WlDataOffer};
    use wayland_client::protocol::wl_registry::WlRegistry;
    use wayland_client::protocol::wl_seat::WlSeat;
    use wayland_client::{Connection, Dispatch, Proxy, QueueHandle, event_created_child};

    use super::Dropped;

    const URI_LIST: &str = "text/uri-list";

    /// Listens for drops on a thread of its own, on winit's connection;
    /// `None` when the window isn't on Wayland.
    pub fn listen(cc: &eframe::CreationContext<'_>) -> Result<Option<Receiver<Dropped>>, String> {
        let RawDisplayHandle::Wayland(handle) = cc.display_handle().map_err(|e| e.to_string())?.as_raw() else {
            return Ok(None);
        };
        // Safety: winit keeps the display open as long as the window, which outlives the app.
        let backend = unsafe { Backend::from_foreign_display(handle.display.as_ptr().cast()) };
        let conn = Connection::from_backend(backend);
        let (globals, mut queue) = registry_queue_init::<State>(&conn).map_err(|e| e.to_string())?;
        let qh = queue.handle();
        let manager: WlDataDeviceManager = globals.bind(&qh, 1..=3, ()).map_err(|e| e.to_string())?;
        let seat: WlSeat = globals.bind(&qh, 1..=1, ()).map_err(|e| e.to_string())?;
        manager.get_data_device(&seat, &qh, ());
        let (tx, rx) = mpsc::channel();
        let mut state = State { tx, ctx: cc.egui_ctx.clone(), conn, offers: HashMap::new(), current: None, at: None };
        std::thread::Builder::new()
            .name("drops".into())
            .spawn(move || while queue.blocking_dispatch(&mut state).is_ok() {})
            .map_err(|e| e.to_string())?;
        Ok(Some(rx))
    }

    struct State {
        tx: Sender<Dropped>,
        ctx: egui::Context,
        conn: Connection,
        /// Each offer's MIME types.
        offers: HashMap<ObjectId, Vec<String>>,
        /// The offer being dragged over the window.
        current: Option<WlDataOffer>,
        /// Where it is, in logical pixels.
        at: Option<egui::Pos2>,
    }

    impl State {
        fn drop(&mut self) -> Option<Dropped> {
            let offer = self.current.take()?;
            let (read, write) = std::io::pipe().ok()?;
            offer.receive(URI_LIST.into(), std::os::fd::AsFd::as_fd(&write));
            drop(write);
            let _ = self.conn.flush();
            let mut list = String::new();
            let _ = (&read).read_to_string(&mut list);
            if offer.version() >= 3 {
                offer.finish();
            }
            offer.destroy();
            self.offers.remove(&offer.id());
            Some(Dropped { at: self.at.take(), paths: super::uri_list(&list) })
        }
    }

    impl Dispatch<WlDataDevice, ()> for State {
        fn event(state: &mut Self, _: &WlDataDevice, event: wl_data_device::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
            match event {
                wl_data_device::Event::Enter { serial, x, y, id: Some(offer), .. } => {
                    let files = state.offers.get(&offer.id()).is_some_and(|m| m.iter().any(|m| m == URI_LIST));
                    offer.accept(serial, files.then(|| URI_LIST.to_string()));
                    if offer.version() >= 3 {
                        let action = if files { DndAction::Copy } else { DndAction::empty() };
                        offer.set_actions(action, action);
                    }
                    state.at = Some(egui::pos2(x as f32, y as f32));
                    state.current = files.then_some(offer);
                }
                wl_data_device::Event::Motion { x, y, .. } => state.at = Some(egui::pos2(x as f32, y as f32)),
                wl_data_device::Event::Leave => {
                    if let Some(offer) = state.current.take() {
                        state.offers.remove(&offer.id());
                        offer.destroy();
                    }
                }
                wl_data_device::Event::Drop => {
                    if let Some(dropped) = state.drop() {
                        let _ = state.tx.send(dropped);
                        state.ctx.request_repaint();
                    }
                }
                // The selection is the clipboard, egui's business.
                wl_data_device::Event::Selection { id: Some(offer) } => {
                    state.offers.remove(&offer.id());
                    offer.destroy();
                }
                _ => {}
            }
        }

        event_created_child!(State, WlDataDevice, [
            wl_data_device::EVT_DATA_OFFER_OPCODE => (WlDataOffer, ()),
        ]);
    }

    impl Dispatch<WlDataOffer, ()> for State {
        fn event(state: &mut Self, offer: &WlDataOffer, event: wl_data_offer::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
            if let wl_data_offer::Event::Offer { mime_type } = event {
                state.offers.entry(offer.id()).or_default().push(mime_type);
            }
        }
    }

    impl Dispatch<WlRegistry, GlobalListContents> for State {
        fn event(_: &mut Self, _: &WlRegistry, _: wayland_client::protocol::wl_registry::Event, _: &GlobalListContents, _: &Connection, _: &QueueHandle<Self>) {}
    }

    impl Dispatch<WlDataDeviceManager, ()> for State {
        fn event(_: &mut Self, _: &WlDataDeviceManager, _: <WlDataDeviceManager as Proxy>::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
    }

    impl Dispatch<WlSeat, ()> for State {
        fn event(_: &mut Self, _: &WlSeat, _: <WlSeat as Proxy>::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_paths_go_as_they_are_others_quoted() {
        let paths = [PathBuf::from("/tmp/shot.png"), PathBuf::from("/home/me/My Pictures/it's.png")];
        assert_eq!(typed(&paths), r"/tmp/shot.png '/home/me/My Pictures/it'\''s.png' ");
    }

    #[test]
    fn a_uri_list_gives_decoded_file_paths() {
        let list = "# comment\r\nfile:///home/me/My%20Pictures/a.png\r\nfile://host/b%2Fc.png\r\nhttps://example.com/x.png\r\n";
        assert_eq!(uri_list(list), [PathBuf::from("/home/me/My Pictures/a.png"), PathBuf::from("/b/c.png")]);
    }
}
