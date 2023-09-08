use std::fs::OpenOptions;
use std::io;

use evdev_rs::Device;
use evdev_rs::InputEvent;
use evdev_rs::ReadFlag;
use evdev_rs::ReadStatus;
use evdev_rs::UInputDevice;

fn on_event(evt: InputEvent) -> Option<InputEvent> {
    Some(evt)
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

    loop {
        let ev = d.next_event(ReadFlag::BLOCKING);
        match ev {
            Ok((ReadStatus::Success, ev)) => {
                // We search for event_type = EV_KEY,
                // event_code = KEY_LEFTMETA, KEY_RIGHTMETA, KEY_TAB.
                // Value is 1 for press and 0 for release
                if let Some(ev) = on_event(ev) {
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
