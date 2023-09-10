use std::fs::OpenOptions;
use std::io;

use evdev_rs::enums::EventCode::EV_KEY;
use evdev_rs::enums::EV_KEY::{KEY_LEFTMETA, KEY_RIGHTMETA, KEY_TAB};
use evdev_rs::Device;
use evdev_rs::InputEvent;
use evdev_rs::ReadFlag;
use evdev_rs::ReadStatus;
use evdev_rs::UInputDevice;

struct AltTabInterceptor {
    was_tab: bool,
    meta_pressed: bool,
}

impl AltTabInterceptor {
    fn new() -> Self {
        Self {
            was_tab: false,
            meta_pressed: false,
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
                    println!("END_META event");
                    self.was_tab = false;
                }
                Some(evt)
            }
            (EV_KEY(KEY_TAB), 1) => {
                if self.meta_pressed {
                    if !self.was_tab {
                        self.was_tab = true;
                    }
                    println!("TAB event");
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
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/input/event12") // TODO: don't hardcode
        .unwrap();
    let mut d = Device::new_from_file(file).unwrap();
    d.grab(evdev_rs::GrabMode::Grab).unwrap();

    let ud = UInputDevice::create_from_device(&d).unwrap();
    println!("uinput device: {}", ud.devnode().unwrap_or("none"));

    let mut intercept = AltTabInterceptor::new();

    loop {
        let ev = d.next_event(ReadFlag::BLOCKING);
        match ev {
            Ok((ReadStatus::Success, ev)) => {
                if let Some(ev) = intercept.on_event(ev) {
                    ud.write_event(&ev).unwrap();
                }
            }
            Ok((ReadStatus::Sync, _)) => {
                println!("Warning: there's no support for SYN_DROPPED yet, ignoring...")
            }
            Err(e) => {
                if e.kind() != io::ErrorKind::WouldBlock {
                    panic!("Error: {}", e);
                }
            }
        }
    }
}
