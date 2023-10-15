use std::error::Error;
use std::os::fd::AsRawFd;

use clap::Parser;

mod interceptor;
mod switcher;

use interceptor::{AltTabInterceptor, KeyConfig};
use switcher::AltTabWorkspaceSwitcher;

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

pub enum WorkspaceSwitcherEvent {
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
                return Err(format!("sway IPC listener error: {e}").into());
            }
            _ => {}
        }
    }

    panic!("Sway IPC connection has been closed");
}
