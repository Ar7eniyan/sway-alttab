use std::fs::OpenOptions;
use std::io;
use std::sync::mpsc::Sender;

use evdev_rs::enums::EventCode::EV_KEY;
use evdev_rs::enums::EV_KEY::{KEY_LEFTMETA, KEY_RIGHTMETA, KEY_TAB};
use evdev_rs::Device;
use evdev_rs::InputEvent;
use evdev_rs::ReadFlag;
use evdev_rs::ReadStatus;
use evdev_rs::UInputDevice;

enum AltTabEvent {
    Tab,
    EndMeta,
}

struct AltTabInterceptor {
    in_device: Device,
    out_device: UInputDevice,
    evt_tx: Sender<AltTabEvent>,
    was_tab: bool,
    meta_pressed: bool,
}

impl AltTabInterceptor {
    fn new(in_device_path: &str, evt_tx: Sender<AltTabEvent>) -> io::Result<Self> {
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
                    self.evt_tx.send(AltTabEvent::EndMeta).unwrap();
                    self.was_tab = false;
                }
                Some(evt)
            }
            (EV_KEY(KEY_TAB), 1) => {
                if self.meta_pressed {
                    self.was_tab = true;
                    self.evt_tx.send(AltTabEvent::Tab).unwrap();
                    None
                } else {
                    Some(evt)
                }
            }
            _ => Some(evt),
        }
    }
}

fn main() {
    let (tx, rx) = std::sync::mpsc::channel::<AltTabEvent>();

    let mut interceptor = AltTabInterceptor::new("/dev/input/event12", tx).unwrap();
    println!(
        "uinput device: {}",
        interceptor.out_device.devnode().unwrap_or("none")
    );

    std::thread::Builder::new()
        .name("interceptor".to_string())
        .spawn(move || interceptor.run())
        .unwrap();

    // Should I do something else here?
    std::thread::park();
}
