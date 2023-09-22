use std::collections::VecDeque;
use std::error::Error;
use std::fs::OpenOptions;
use std::io;
use std::sync::mpsc::{Receiver, Sender};

use evdev_rs::enums::EventCode::EV_KEY;
use evdev_rs::enums::EV_KEY::{KEY_LEFTMETA, KEY_RIGHTMETA, KEY_TAB};
use evdev_rs::Device;
use evdev_rs::InputEvent;
use evdev_rs::ReadFlag;
use evdev_rs::ReadStatus;
use evdev_rs::UInputDevice;

enum WorkspaceSwitcherEvent {
    Tab,
    EndMeta,
    SwayWsEvent(Box<swayipc::WorkspaceEvent>),
}

impl std::fmt::Debug for WorkspaceSwitcherEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Tab => f.write_str("Tab"),
            Self::EndMeta => f.write_str("EndMeta"),
            Self::SwayWsEvent(evt) => {
                // Default debug output for WorkspaceEvent is too large, display only the change type
                f.write_fmt(format_args!("SwayWsEvent({:?})", evt.as_ref().change))
            }
        }
    }
}

struct AltTabInterceptor {
    in_device: Device,
    out_device: UInputDevice,
    evt_tx: Sender<WorkspaceSwitcherEvent>,
    was_tab: bool,
    meta_pressed: bool,
}

impl AltTabInterceptor {
    fn new(
        in_device_path: &str,
        evt_tx: Sender<WorkspaceSwitcherEvent>,
    ) -> Result<Self, Box<dyn Error>> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(in_device_path)
            .map_err(|e| {
                format!("can't open keyboard input device file ({in_device_path}): {e}")
            })?;

        let mut in_device = Device::new_from_file(file)
            .map_err(|e| format!("can't create libevdev input device: {e}"))?;
        in_device
            .grab(evdev_rs::GrabMode::Grab)
            .map_err(|e| format!("can't grab the input device: {e}"))?;
        let out_device = UInputDevice::create_from_device(&in_device)
            .map_err(|e| format!("can't create a uinput device: {e}"))?;

        log::debug!("Initialized the keypress interceptor");
        log::debug!("Keyboard input device: {in_device_path}");
        log::debug!(
            "UInput device devnode: {}, syspath: {}",
            out_device.devnode().unwrap_or("none"),
            out_device.syspath().unwrap_or("none")
        );

        Ok(Self {
            in_device,
            out_device,
            evt_tx,
            was_tab: false,
            meta_pressed: false,
        })
    }

    fn run(&mut self) {
        log::info!("Starting the keypress interceptor...");

        loop {

            let ev = self.in_device.next_event(ReadFlag::BLOCKING);
            match ev {
                Ok((ReadStatus::Success, ev)) => {
                    if let Some(ev) = self.on_event(ev) {
                        self.out_device
                            .write_event(&ev)
                            .expect("error writing to the uinput device");
                    }
                }
                Ok((ReadStatus::Sync, _)) => {
                    log::warn!("There's no support for SYN_DROPPED yet, ignoring");
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                    log::warn!("next_event() should block, something is wrong");
                }
                Err(_) => {
                    ev.expect("error reading from the input device");
                }
            }
        }
    }

    // This function is called on each event got from a configured input device.
    // The return value is an optional event to send to the fake uinput device.
    fn on_event(&mut self, evt: InputEvent) -> Option<InputEvent> {
        // evt.value in EV_KEY is 0 for release, 1 for press and 2 for hold.
        match (evt.event_code, evt.value) {
            (EV_KEY(KEY_LEFTMETA) | EV_KEY(KEY_RIGHTMETA), 0 | 1) => {
                self.meta_pressed = evt.value == 1;
                if evt.value == 0 && self.was_tab {
                    self.evt_tx
                        .send(WorkspaceSwitcherEvent::EndMeta)
                        .expect("can't send a key event, channel is dead");
                    self.was_tab = false;
                }
                Some(evt)
            }
            (EV_KEY(KEY_TAB), 1) => {
                if self.meta_pressed {
                    self.was_tab = true;
                    self.evt_tx
                        .send(WorkspaceSwitcherEvent::Tab)
                        .expect("can't send a key event, channel is dead");
                    None
                } else {
                    Some(evt)
                }
            }
            _ => Some(evt),
        }
    }
}

struct AltTabWorkspaceSwitcher {
    evt_rx: Receiver<WorkspaceSwitcherEvent>,
    // Sway IPC connection
    sway_ipc: swayipc::Connection,
    // Workspace IDs in the most to least recently used order
    mru_workspaces: VecDeque<i64>,
    // Count of tab keypresses in a row, zero means the tab sequence is not triggered
    // Always a valid index for mru_workspaces
    tab_count: usize,
}

impl AltTabWorkspaceSwitcher {
    fn new(evt_rx: Receiver<WorkspaceSwitcherEvent>) -> Self {
        let sway_ipc =
            swayipc::Connection::new().expect("sway IPC socket should be available for connection");

        log::debug!("Initialized workspace switcher");

        Self {
            evt_rx,
            sway_ipc,
            mru_workspaces: VecDeque::new(),
            tab_count: 0,
        }
    }

    fn run(&mut self) {
        log::info!("Starting the workspace switcher...");

        loop {
            let evt = self.evt_rx.recv().expect("can't read from event channel");
            log::debug!("Processing event: {:?}", evt);

            match evt {
                WorkspaceSwitcherEvent::Tab => {
                    if self.mru_workspaces.is_empty() {
                        continue;
                    }

                    // Switch to the next workspace, wrapping around if currently at the end
                    self.tab_count = (self.tab_count + 1) % self.mru_workspaces.len();
                    let tree = self
                        .sway_ipc
                        .get_tree()
                        .expect("can't get container tree via sway IPC");
                    let ws_name = Self::workspace_name_by_id(&tree, self.mru_workspaces[self.tab_count])
                        .expect("the id should be associated with an existing workspace (MRU list is probably not in sync)");
                    self.sway_ipc
                        .run_command(format!("workspace {}", ws_name))
                        .expect("can't switch workspace using sway IPC command");
                }
                WorkspaceSwitcherEvent::EndMeta => {
                    if self.mru_workspaces.is_empty() {
                        continue;
                    }
                    self.end_sequence(self.mru_workspaces[self.tab_count]);
                }
                WorkspaceSwitcherEvent::SwayWsEvent(ws_event) => {
                    self.handle_ws_event(ws_event.as_ref());
                }
            }

            log::debug!("MRU list: {}", self.format_mru_list());
        }
    }

    fn workspace_name_by_id(tree: &swayipc::Node, id: i64) -> Option<&str> {
        tree.nodes
            .iter()
            .flat_map(|output| output.nodes.iter())
            .find(|workspace| workspace.id == id)?
            .name
            .as_deref()
    }

    fn end_sequence(&mut self, new_ws_id: i64) {
        if self.tab_count == 0 {
            return;
        }
        self.mru_workspaces.retain(|&id| id != new_ws_id);
        self.mru_workspaces.push_front(new_ws_id);
        self.tab_count = 0;
    }

    // Reduces code nesting
    #[allow(clippy::comparison_chain)]
    fn handle_ws_event(&mut self, ws_event: &swayipc::WorkspaceEvent) {
        // Sway workspace event types:
        // init - add the to the end of the list
        // empty - remove from the list
        // focus - move to the beginning of the list
        // move, rename, urgent, reload - ignore

        // All events we're interested in have `current` workspace field
        if let Some(current_id) = ws_event.current.as_ref().map(|x| x.id) {
            match ws_event.change {
                swayipc::WorkspaceChange::Init => {
                    self.mru_workspaces.push_back(current_id);
                }
                swayipc::WorkspaceChange::Empty => {
                    if let Some(idx) = self.mru_workspaces.iter().position(|&x| x == current_id) {
                        self.mru_workspaces.remove(idx);
                        if idx < self.tab_count {
                            self.tab_count -= 1;
                        } else if idx == self.tab_count {
                            // TODO: should we recover properly?
                            panic!("Error: the currently focused workspace is deleted");
                        }
                    } else {
                        log::warn!("Deleting unlisted workspace");
                    }
                }
                swayipc::WorkspaceChange::Focus => {
                    if self.tab_count != 0 && current_id != self.mru_workspaces[self.tab_count] {
                        // Workspace switch not caused by a tab press, stop the sequence
                        self.end_sequence(current_id);
                    }

                    if self.tab_count == 0 {
                        self.mru_workspaces.retain(|&x| x != current_id);
                        self.mru_workspaces.push_front(current_id);
                    }
                }
                _ => {}
            }
        }
    }

    // For debugging purposes
    fn format_mru_list(&mut self) -> String {
        let tree = self
            .sway_ipc
            .get_tree()
            .expect("can't get container tree via sway IPC");
        format!(
            "{:?}",
            self.mru_workspaces
                .iter()
                .map(|&id| Self::workspace_name_by_id(&tree, id))
                .collect::<Vec<_>>()
        )
    }
}

fn main() {
    env_logger::builder()
        .filter_level(log::LevelFilter::Info)
        .parse_default_env()
        .init();

    let (tx, rx) = std::sync::mpsc::channel::<WorkspaceSwitcherEvent>();

    // When user presses enter to run this program in a terminal, the press
    // event is sent from the real keyboard, but the release event is sent
    // from the fake uinput device, creating a weird behavior of spamming enter.
    // The delay is to make sure the release event is sent correctly.
    // TODO: make the delay optional
    std::thread::sleep(std::time::Duration::from_millis(250));

    let mut interceptor = AltTabInterceptor::new("/dev/input/event12", tx.clone()).unwrap();

    std::thread::Builder::new()
        .name("workspace-switcher".to_string())
        .spawn(move || AltTabWorkspaceSwitcher::new(rx).run())
        .expect("can't create workspace switcher thread");

    std::thread::Builder::new()
        .name("interceptor".to_string())
        .spawn(move || interceptor.run())
        .expect("can't create keypress interceptor thread");

    let conn =
        swayipc::Connection::new().expect("sway IPC socket should be available for connection");
    let evt_iter = conn
        .subscribe([swayipc::EventType::Workspace])
        .expect("can't subscribe to sway IPC workspace events");

    // Forward sway workspace events to the switcher thread
    for evt in evt_iter {
        match evt {
            Ok(swayipc::Event::Workspace(evt)) => {
                tx.send(WorkspaceSwitcherEvent::SwayWsEvent(evt))
                    .expect("can't send a sway workspace event, the channel is dead");
            }
            Err(e) => {
                println!("Sway event stream error: {:?}", e);
            }
            _ => {}
        }
    }

    panic!("Sway IPC connection has been closed");
}
