use std::error::Error;
use std::sync::mpsc::Sender;

use evdev_rs::enums::EventCode::EV_KEY;
use evdev_rs::{Device, InputEvent, ReadFlag, ReadStatus, UInputDevice};

use super::WorkspaceSwitcherEvent;

pub struct KeyConfig {
    // To avoid searching in Vec<EV_KEY>, there is one required modifier and one optional
    // Guess it helps with performance (remember, we're filtering realtime keyboard events)
    pub modifier1: evdev_rs::enums::EV_KEY,
    pub modifier2: Option<evdev_rs::enums::EV_KEY>,
    pub trigger: evdev_rs::enums::EV_KEY,
}

pub struct AltTabInterceptor {
    in_device: Device,
    out_device: UInputDevice,
    evt_tx: Sender<WorkspaceSwitcherEvent>,
    key_config: KeyConfig,
    was_tab: bool,
    meta_pressed: bool,
}

impl AltTabInterceptor {
    pub fn new(
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

    pub fn run(&mut self) {
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
