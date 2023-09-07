use std::fs::OpenOptions;
use std::io::ErrorKind;

use evdev_rs::Device;
use evdev_rs::ReadFlag;
use evdev_rs::ReadStatus;

fn main() {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/input/event12")  // TODO: don't hardcode
        .unwrap();
    let d = Device::new_from_file(file).unwrap();

    loop {
        let ev = d.next_event(ReadFlag::BLOCKING);
        match ev {
            Ok((ReadStatus::Success, ev)) => println!(
                "Event: time {}.{}, +++ {}, {}, {} +++",
                ev.time.tv_sec,
                ev.time.tv_usec,
                ev.event_type()
                    .map(|ev_type| format!("{}", ev_type))
                    .unwrap_or("".to_owned()),
                ev.event_code,
                ev.value
            ),
            Ok((ReadStatus::Sync, ev)) => println!("Sync: {}", ev.event_type().unwrap()),
            Err(e) => {
                if e.kind() != ErrorKind::WouldBlock {
                    panic!("Error: {}", e);
                }
            }
        }
    }
}
