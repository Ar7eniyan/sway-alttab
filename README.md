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

To use this program, you'll need to find the input device file for your keyboard. Input device files are placed by the Linux kernel in `/dev/input/eventN`. There are some tools to show the available input devices: for example `sudo evtest` or `sudo libinput debug-events`. Without using any third-party software, the names and other attributes of input devices can be found by `cat /proc/bus/input/devices`.

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
The actual shortcut is not Alt+Tab, but `(LMeta|RMeta)+Tab`, following the default Sway usage of Meta key for navigation. Focus on different workspaces for the tool to start keeping track of them, and press the key combination to see if it works.

To run `swaywm-alttab` on sway startup, add the following line to `~/.config/sway/config`:
```
exec ~/.cargo/bin/swaywm-alttab /dev/input/eventN
```

### Custom key combination

The default key combination is `(LMeta|RMeta)+Tab`, remember that. But if you want, you can configure any key combination by using command line `--modifiers` and `--trigger` options. For example, the default setup would look like this if redundantly configured with the mentioned options: `swaywm-alttab --modifiers KEY_LEFTMETA KEY_RIGHTMETA --trigger KEY_TAB <input device>`. The app supports setting 1 or 2 modifier keys, and exactly one trigger key if you need to change it for some reason. To use the Alt+Tab shortcut (like on most platforms) instead of Meta+Tab, run the app like this:
```
~/.cargo/bin/swaywm-alttab <input device> --modifiers KEY_LEFTALT
```
> [!WARNING]
> Be careful when passing `--modifiers` option since it takes up to two values, which would mistakenly try to parse the path as a key name in this case:
> ```
> ~/.cargo/bin/swaywm-alttab --modifiers KEY_LEFTALT <input device>
> error: invalid value '<input device>' for '--modifiers <MODIFIERS>...': no such key code
> ```

## Debugging

To enable logging, set environment variable RUST_LOG to one of these values: error, warn, info, debug, trace. The default log level is info. For more complex selectors, see [env_logger](https://docs.rs/env_logger/latest/env_logger/#enabling-logging)'s documentation.

## Further development

- [ ] Find a more convinient way to switch workspaces (ideally, by their con_id)
- [X] Make a key combination configurable
- [ ] Implement input device autodetection if it's possible
- [ ] Add workspace overview UI like in [sov](https://github.com/milgra/sov)

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