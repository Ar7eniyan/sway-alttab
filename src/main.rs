use std::collections::VecDeque;
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

#[derive(Debug)]
enum WorkspaceSwitcherEvent {
    Tab,
    EndMeta,
    SwayWsEvent(Box<swayipc::WorkspaceEvent>),
}

struct AltTabInterceptor {
    in_device: Device,
    out_device: UInputDevice,
    evt_tx: Sender<WorkspaceSwitcherEvent>,
    was_tab: bool,
    meta_pressed: bool,
}

impl AltTabInterceptor {
    fn new(in_device_path: &str, evt_tx: Sender<WorkspaceSwitcherEvent>) -> io::Result<Self> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(in_device_path)?;

        let mut in_device = Device::new_from_file(file)?;
        in_device.grab(evdev_rs::GrabMode::Grab)?;
        let out_device = UInputDevice::create_from_device(&in_device)?;

        Ok(Self {
            in_device,
            out_device,
            evt_tx,
            was_tab: false,
            meta_pressed: false,
        })
    }

    fn run(&mut self) {
        loop {
            let ev = self.in_device.next_event(ReadFlag::BLOCKING);
            match ev {
                Ok((ReadStatus::Success, ev)) => {
                    if let Some(ev) = self.on_event(ev) {
                        self.out_device.write_event(&ev).unwrap();
                    }
                }
                Ok((ReadStatus::Sync, _)) => {
                    println!("Warning: there's no support for SYN_DROPPED yet, ignoring...")
                }
                Err(ref e) => {
                    if e.kind() != io::ErrorKind::WouldBlock {
                        ev.unwrap();
                    }
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
                    self.evt_tx.send(WorkspaceSwitcherEvent::EndMeta).unwrap();
                    self.was_tab = false;
                }
                Some(evt)
            }
            (EV_KEY(KEY_TAB), 1) => {
                if self.meta_pressed {
                    self.was_tab = true;
                    self.evt_tx.send(WorkspaceSwitcherEvent::Tab).unwrap();
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
    // Workspace IDs in the most to least recently used order
    mru_workspaces: VecDeque<i64>,
    // Count of tab keypresses in a row, zero means the tab sequence is not triggered
    // Always a valid index for mru_workspaces
    tab_count: usize,
}

impl AltTabWorkspaceSwitcher {
    fn new(evt_rx: Receiver<WorkspaceSwitcherEvent>) -> Self {
        Self {
            evt_rx,
            mru_workspaces: VecDeque::new(),
            tab_count: 0,
        }
    }

    fn run(&mut self) {
        loop {
            let evt = self.evt_rx.recv().unwrap();
            // println!("Got event: {:#?}", evt);

            match evt {
                WorkspaceSwitcherEvent::Tab => {
                    // Switch to the next workspace, wrapping around if currently at the end
                    self.tab_count = (self.tab_count + 1) % self.mru_workspaces.len();
                    // TODO: send sway ipc command to change workspace
                }
                WorkspaceSwitcherEvent::EndMeta => {
                    self.end_sequence(self.mru_workspaces[self.tab_count]);
                }
                WorkspaceSwitcherEvent::SwayWsEvent(ws_event) => {
                    self.handle_ws_event(ws_event.as_ref());
                }
            }
        }
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
                        println!("Warning: deleting unlisted workspace");
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
}

fn main() {
    // When user presses enter to run this program in a terminal, the press
    // event is sent from the real keyboard, but the release event is sent
    // from the fake uinput device, creating a weird behavior of spamming enter.
    // The delay is to make sure the release event is sent correctly.
    // TODO: make the delay optional
    std::thread::sleep(std::time::Duration::from_millis(250));
    let (tx, rx) = std::sync::mpsc::channel::<WorkspaceSwitcherEvent>();

    let mut interceptor = AltTabInterceptor::new("/dev/input/event12", tx.clone()).unwrap();
    println!(
        "uinput device: {}",
        interceptor.out_device.devnode().unwrap_or("none")
    );

    let mut ws_switcher = AltTabWorkspaceSwitcher::new(rx);

    std::thread::Builder::new()
        .name("workspace-switcher".to_string())
        .spawn(move || ws_switcher.run())
        .unwrap();

    std::thread::Builder::new()
        .name("interceptor".to_string())
        .spawn(move || interceptor.run())
        .unwrap();

    let conn = swayipc::Connection::new().unwrap();
    let evt_iter = conn.subscribe([swayipc::EventType::Workspace]).unwrap();

    // Forward sway workspace events to the switcher thread
    // Should I make this a separate thread?
    for evt in evt_iter {
        match evt {
            Ok(swayipc::Event::Workspace(evt)) => {
                tx.send(WorkspaceSwitcherEvent::SwayWsEvent(evt)).unwrap()
            }
            Err(e) => {
                println!("Sway event stream error: {:?}", e);
            }
            _ => {}
        }
    }

    println!("Sway IPC connections has been closed, exiting...");

    // Should I do something else here?
    // std::thread::park();
}
