use std::collections::VecDeque;
use std::error::Error;
use std::os::fd::AsRawFd;
use std::sync::mpsc::{Receiver, Sender};

use clap::Parser;
use evdev_rs::enums::EventCode::EV_KEY;
use evdev_rs::{Device, InputEvent, ReadFlag, ReadStatus, UInputDevice};

fn parse_keycode(key: &str) -> Result<evdev_rs::enums::EV_KEY, &'static str> {
    <evdev_rs::enums::EV_KEY as std::str::FromStr>::from_str(key).map_err(|_| "no such key code")
}

#[derive(clap::Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Cli {
    // TODO: make optional, try to autodetect if not given
    #[arg(
        help = "The keyboard input device path to use for intercepting keypresses\n\
        (/dev/input/eventN or other)"
    )]
    input_device: std::path::PathBuf,

    #[arg(
        short, long,
        value_parser = parse_keycode,
        num_args = 1..=2,
        value_delimiter = ',',
        default_values = ["KEY_LEFTMETA", "KEY_RIGHTMETA"]
    )]
    /// The first key in the Alt-Tab sequence (modifier), up to 2 options
    modifiers: Vec<evdev_rs::enums::EV_KEY>,

    #[arg(
        short, long,
        value_parser = parse_keycode,
        default_value = "KEY_TAB"
    )]
    /// The second key in the Alt-Tab seqence (trigger)
    trigger: evdev_rs::enums::EV_KEY,
}

enum WorkspaceSwitcherEvent {
    Trigger,
    EndMod,
    SwayWsEvent(Box<swayipc::WorkspaceEvent>),
}

impl std::fmt::Debug for WorkspaceSwitcherEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Trigger => f.write_str("Trigger"),
            Self::EndMod => f.write_str("EndMod"),
            Self::SwayWsEvent(evt) => {
                // Default debug output for WorkspaceEvent is too large, display only the change type
                f.write_fmt(format_args!("SwayWsEvent({:?})", evt.as_ref().change))
            }
        }
    }
}

struct KeyConfig {
    // To avoid searching in Vec<EV_KEY>, there is one required modifier and one optional
    // Guess it helps with performance (remember, we're filtering realtime keyboard events)
    modifier1: evdev_rs::enums::EV_KEY,
    modifier2: Option<evdev_rs::enums::EV_KEY>,
    trigger: evdev_rs::enums::EV_KEY,
}

struct AltTabInterceptor {
    in_device: Device,
    out_device: UInputDevice,
    evt_tx: Sender<WorkspaceSwitcherEvent>,
    key_config: KeyConfig,
    was_tab: bool,
    meta_pressed: bool,
}

impl AltTabInterceptor {
    fn new(
        in_device_path: &std::path::Path,
        evt_tx: Sender<WorkspaceSwitcherEvent>,
        key_config: KeyConfig,
    ) -> Result<Self, Box<dyn Error>> {
        if key_config.trigger == key_config.modifier1
            || Some(key_config.trigger) == key_config.modifier2
        {
            return Err(
                "the modifier keys overlap with the trigger key, check your key configuration"
                    .into(),
            );
        }

        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(in_device_path)
            .map_err(|e| {
                format!(
                    "can't open keyboard input device file ({}): {e}",
                    in_device_path.display()
                )
            })?;

        let mut in_device = Device::new_from_file(file)
            .map_err(|e| format!("can't create libevdev input device: {e}"))?;
        in_device
            .grab(evdev_rs::GrabMode::Grab)
            .map_err(|e| format!("can't grab the input device: {e}"))?;
        let out_device = UInputDevice::create_from_device(&in_device)
            .map_err(|e| format!("can't create a uinput device: {e}"))?;

        log::debug!("Initialized the keypress interceptor");
        log::debug!("Keyboard input device: {}", in_device_path.display());
        log::debug!(
            "UInput device devnode: {}, syspath: {}",
            out_device.devnode().unwrap_or("none"),
            out_device.syspath().unwrap_or("none")
        );

        Ok(Self {
            in_device,
            out_device,
            evt_tx,
            key_config,
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
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
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
            (EV_KEY(mod_), 0 | 1)
                if mod_ == self.key_config.modifier1 || Some(mod_) == self.key_config.modifier2 =>
            {
                self.meta_pressed = evt.value == 1;
                if evt.value == 0 && self.was_tab {
                    self.evt_tx
                        .send(WorkspaceSwitcherEvent::EndMod)
                        .expect("can't send a key event, channel is dead");
                    self.was_tab = false;
                }
                Some(evt)
            }
            (EV_KEY(trig), 1) if trig == self.key_config.trigger => {
                if self.meta_pressed {
                    self.was_tab = true;
                    self.evt_tx
                        .send(WorkspaceSwitcherEvent::Trigger)
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
                WorkspaceSwitcherEvent::Trigger => {
                    if self.mru_workspaces.is_empty() {
                        continue;
                    }

                    // Switch to the next workspace, wrapping around if currently at the end
                    self.tab_count = (self.tab_count + 1) % self.mru_workspaces.len();
                    self.switch_to_workspace(self.mru_workspaces[self.tab_count]);
                }
                WorkspaceSwitcherEvent::EndMod => {
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

    fn switch_to_workspace(&mut self, id: i64) {
        let tree = self
            .sway_ipc
            .get_tree()
            .expect("can't get container tree via sway IPC");
        let ws_name = Self::workspace_name_by_id(&tree, id)
            .expect("the id should be associated with an existing workspace (MRU list is probably not in sync)");

        log::debug!(
            "Focusing on workspace with id = {}, name = \"{}\"",
            id,
            ws_name
        );

        self.sway_ipc
            .run_command(format!("workspace {}", ws_name))
            .expect("can't switch workspace using sway IPC command")[0]
            // the only command is `workspace`, its result is at index 0
            .as_ref()
            .expect("can't switch workspace using sway IPC command");
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
                            panic!("the currently focused workspace is deleted");
                        }
                    } else {
                        log::warn!("Deleting unlisted workspace");
                    }
                }
                swayipc::WorkspaceChange::Focus => {
                    if self.tab_count == 0 {
                        self.mru_workspaces.retain(|&x| x != current_id);
                        self.mru_workspaces.push_front(current_id);
                    } else if current_id != self.mru_workspaces[self.tab_count] {
                        // Tab sequence is active and the workspace switch isn't
                        // caused by a tab press, stop the sequence
                        self.end_sequence(current_id);
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

fn main() -> Result<(), Box<dyn Error>> {
    env_logger::builder()
        .filter_level(log::LevelFilter::Info)
        .parse_default_env()
        .init();

    let cli = Cli::parse();
    log::debug!("Parsed arguments: {:#?}", cli);
    let (tx, rx) = std::sync::mpsc::channel::<WorkspaceSwitcherEvent>();

    // When user presses enter to run this program in a terminal, the press
    // event is sent from the real keyboard, but the release event is sent
    // from the fake uinput device, creating a stream of repeated enter presses.
    // The delay is to make sure the release event is sent correctly.
    let interactive = unsafe { libc::isatty(std::io::stdin().as_raw_fd()) == 1 };
    if interactive {
        log::debug!("Performing a 500ms delay because running interactively...");
        std::thread::sleep(std::time::Duration::from_millis(500));
    }

    let input_device_path = cli.input_device;
    let mut interceptor = AltTabInterceptor::new(
        &input_device_path,
        tx.clone(),
        KeyConfig {
            modifier1: cli.modifiers[0],
            modifier2: cli.modifiers.get(1).copied(),
            trigger: cli.trigger,
        },
    )?;

    std::thread::Builder::new()
        .name("workspace-switcher".to_string())
        .spawn(move || AltTabWorkspaceSwitcher::new(rx).run())
        .map_err(|e| format!("can't create workspace switcher thread: {e}"))?;

    std::thread::Builder::new()
        .name("interceptor".to_string())
        .spawn(move || interceptor.run())
        .map_err(|e| format!("can't create keypress interceptor thread: {e}"))?;

    let conn = swayipc::Connection::new()
        .map_err(|e| format!("sway IPC socket should be available for connection: {e}"))?;
    let evt_iter = conn
        .subscribe([swayipc::EventType::Workspace])
        .map_err(|e| format!("can't subscribe to sway IPC workspace events: {e}"))?;

    // Forward sway workspace events to the switcher thread
    for evt in evt_iter {
        match evt {
            Ok(swayipc::Event::Workspace(evt)) => {
                tx.send(WorkspaceSwitcherEvent::SwayWsEvent(evt))
                    .map_err(|e| {
                        format!("can't send a sway workspace event, the channel is dead: {e}")
                    })?;
            }
            Err(e) => {
                println!("Sway event stream error: {:?}", e);
            }
            _ => {}
        }
    }

    panic!("Sway IPC connection has been closed");
}
