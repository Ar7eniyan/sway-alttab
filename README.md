# swaywm-alttab

A tool which brings familiar Alt-Tab shortcut from stacking window managers (used in Windows, Mac, KDE, GNOME, etc.) to Sway WM workspaces on Linux

## Installation

Install a binary crate with cargo:
```
cargo install --git https://github.com/ar7eniyan/swaywm-alttab
```
The `swaywm-alttab` binary is going to be in `~/.cargo/bin`, add it to `PATH` if needed

TODO: make an AUR package.

## Configuration

To use this program, you'll need to find the input device file for your keyboard. Input device files are placed by the Linux kernel in `/dev/input/eventN`. There are some tools to show the available input devices: for example `evtest` or `libinput debug-events`. Without using any third-party software, the names and other attributes of input devices can be found by `cat /proc/bus/input/devices`.

### Seting up the permissions for keyboard input/output to work

In order for `swaywm-alttab` to properly access input devices and uinput kernel device as an unpriviliged user, the following steps should be made:
- Create udev rules for `/dev/input/` and `/dev/uinput`:
    -  `/etc/udev/rules.d/72-swaywm-alttab-input.rules`:
        ```
        KERNEL=="uinput", MODE="0660", TAG+="uaccess"
        SUBSYSTEM=="input", MODE="0660", TAG+="uaccess"
        ```
        TODO: does this put a security risk on a system?
- Make `uinput` kernel module to load on boot (doesn't load automatically at least on Arch):
    - `/etc/modules-load.d/uinput.conf`:
        ```
        uinput
        ```
- Reboot or run the following:
    ```
    # Reload udev rules
    sudo udevadm trigger
    sudo udevadm control --reload
    # Load uinput kernel module
    sudo modprobe uinput
    ```

## Usage

After you found the `/dev/input/eventN` path for your keyboard and set up the permissions, start the tool in a terminal to check if everything works correctly. Pass the input device path as an argument, replacing `N` with yout actual device number:
```
~/.cargo/bin/swaywm-alttab /dev/input/eventN
```
Focus on different workspaces for the tool to start keeping track of them, and press Alt-Tab shortcut to see if it works.

To run `swaywm-alttab` on sway startup, add the following line to `~/.config/sway/config`:
```
exec ~/.cargo/bin/swaywm-alttab /dev/input/eventN
```

## Debugging

To enable logging, set environment variable RUST_LOG to one of these values: error, warn, info, debug, trace. The default log level is info. For more complex selectors, see [env_logger](https://docs.rs/env_logger/latest/env_logger/#enabling-logging)'s documentation.

## Further development

- Implement input device autodetection if it's possible
- Add workspace overview UI like in [sov](https://github.com/milgra/sov)

## License: GNU GPLv3, see [COPYING](COPYING)

```
Copyright (C) 2023  Arseniy Kuznetsov

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU General Public License as published by
the Free Software Foundation, either version 3 of the License, or
(at your option) any later version.

This program is distributed in the hope that it will be useful,
but WITHOUT ANY WARRANTY; without even the implied warranty of
MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
GNU General Public License for more details.

You should have received a copy of the GNU General Public License
along with this program.  If not, see <https://www.gnu.org/licenses/>.
```