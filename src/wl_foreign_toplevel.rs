use std::collections::HashMap;
use std::{sync::Arc, thread, time};

use crate::{wl_connection::subscribe_state, Watcher};

use super::report_client::ReportClient;
use super::wl_bindings;
use super::wl_connection::WlEventConnection;
use super::BoxedError;
use chrono::{DateTime, Utc};
use wayland_client::{
    event_created_child, globals::GlobalListContents, protocol::wl_registry, Connection, Dispatch,
    Proxy, QueueHandle,
};
use wl_bindings::wlr_foreign_toplevel::zwlr_foreign_toplevel_handle_v1::{
    Event as HandleEvent, State as HandleState, ZwlrForeignToplevelHandleV1,
};
use wl_bindings::wlr_foreign_toplevel::zwlr_foreign_toplevel_manager_v1::{
    Event as ManagerEvent, ZwlrForeignToplevelManagerV1, EVT_TOPLEVEL_OPCODE,
};

struct WindowData {
    app_id: String,
    title: String,
}

struct ToplevelState {
    windows: HashMap<String, WindowData>,
    current_window_id: Option<String>,
    _last_input_time: DateTime<Utc>,
    _is_idle: bool,
    _is_changed: bool,
    client: Arc<ReportClient>,
}

impl ToplevelState {
    fn new(client: Arc<ReportClient>) -> Self {
        Self {
            windows: HashMap::new(),
            current_window_id: None,
            _last_input_time: Utc::now(),
            _is_idle: false,
            _is_changed: false,
            client,
        }
    }
}

impl Dispatch<ZwlrForeignToplevelManagerV1, ()> for ToplevelState {
    fn event(
        state: &mut Self,
        _: &ZwlrForeignToplevelManagerV1,
        event: <ZwlrForeignToplevelManagerV1 as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            ManagerEvent::Toplevel { toplevel } => {
                debug!("Toplevel handle is received {}", toplevel.id());
                state.windows.insert(
                    toplevel.id().to_string(),
                    WindowData {
                        app_id: "unknown".into(),
                        title: "unknown".into(),
                    },
                );
            }
            ManagerEvent::Finished => {
                error!("Toplevel manager is finished, the application may crash");
            }
        };
    }

    event_created_child!(ToplevelState, ZwlrForeignToplevelManagerV1, [
        EVT_TOPLEVEL_OPCODE => (ZwlrForeignToplevelHandleV1, ()),
    ]);
}

subscribe_state!(wl_registry::WlRegistry, GlobalListContents, ToplevelState);
subscribe_state!(wl_registry::WlRegistry, (), ToplevelState);

impl Dispatch<ZwlrForeignToplevelHandleV1, ()> for ToplevelState {
    fn event(
        toplevel_state: &mut Self,
        handle: &ZwlrForeignToplevelHandleV1,
        event: <ZwlrForeignToplevelHandleV1 as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let id = handle.id().to_string();
        let window = toplevel_state.windows.get_mut(&id);
        if let Some(window) = window {
            match event {
                HandleEvent::Title { title } => {
                    trace!("Title is changed for {id}: {title}");
                    window.title = title;
                }
                HandleEvent::AppId { app_id } => {
                    trace!("App ID is changed for {id}: {app_id}");
                    window.app_id = app_id;
                }
                HandleEvent::State { state } => {
                    trace!("State is changed for {id}: {:?}", state);
                    if state.contains(&(HandleState::Activated as u8)) {
                        trace!("Window is activated: {id}");
                        toplevel_state.current_window_id = Some(id);
                    }
                }
                HandleEvent::Done => trace!("Done: {id}"),
                HandleEvent::Closed => {
                    trace!("Window is closed: {id}");
                    if toplevel_state.windows.remove(&id).is_none() {
                        warn!("Window is already removed: {id}");
                    }
                }
                _ => (),
            };
        } else {
            error!("Window is not found: {id}");
        }
    }
}

impl ToplevelState {
    fn send_active_window(&self) -> Result<(), BoxedError> {
        let active_window_id = self
            .current_window_id
            .as_ref()
            .ok_or("Current window is unknown")?;
        let active_window = self.windows.get(active_window_id).ok_or(format!(
            "Current window is not found by ID {active_window_id}"
        ))?;

        self.client
            .send_active_window(&active_window.app_id, &active_window.title)
            .map_err(|_| "Failed to send heartbeat for active window".into())
    }
}

pub struct WlrForeignToplevelWatcher {
    connection: WlEventConnection<ToplevelState>,
}

impl Watcher for WlrForeignToplevelWatcher {
    fn new() -> Result<Self, BoxedError> {
        let connection: WlEventConnection<ToplevelState> = WlEventConnection::connect()?;
        connection.get_foreign_toplevel_manager()?;

        Ok(Self { connection })
    }

    fn watch(&mut self, client: &Arc<ReportClient>) {
        let mut toplevel_state = ToplevelState::new(Arc::clone(client));

        self.connection
            .event_queue
            .roundtrip(&mut toplevel_state)
            .unwrap();

        info!("Starting wlr foreign toplevel watcher");
        loop {
            if let Err(e) = self.connection.event_queue.roundtrip(&mut toplevel_state) {
                error!("Event queue is not processed: {e}");
            } else if let Err(e) = toplevel_state.send_active_window() {
                error!("Error on idle iteration {e}");
            }

            thread::sleep(time::Duration::from_secs(u64::from(
                client.config.poll_time_window,
            )));
        }
    }
}