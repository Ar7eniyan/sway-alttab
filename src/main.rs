use std::fs::OpenOptions;
use std::io::ErrorKind;

use evdev_rs::Device;
use evdev_rs::ReadFlag;
use evdev_rs::ReadStatus;
use evdev_rs::UInputDevice;

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
                println!(
                    "Event: time {}.{}, +++ {}, {}, {} +++",
                    ev.time.tv_sec,
                    ev.time.tv_usec,
                    ev.event_type()
                        .map(|ev_type| format!("{}", ev_type))
                        .unwrap_or("".to_owned()),
                    ev.event_code,
                    ev.value
                );
                ud.write_event(&ev).unwrap();
            }
            Ok((ReadStatus::Sync, _)) => {
                println!("Warning: there's no support for SYN_DROPPED yet, ignoring...")
            }
            Err(e) => {
                if e.kind() != ErrorKind::WouldBlock {
                    panic!("Error: {}", e);
                }
            }
        }
    }
}
