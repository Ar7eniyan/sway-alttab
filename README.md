###  How to set up the permissions for keyboard input/output to work

- Create udev rules for `/dev/input*` and `/dev/uinput`:
    -  `/etc/udev/rules.d/72-sway-alttab-input.rules`:
        ```
        KERNEL=="uinput", MODE="0660", TAG+="uaccess"
        SUBSYSTEM=="input", MODE="0660", TAG+="uaccess"
        ```
- Make `uinput` kernel module to load on boot (doesn't load automatically at least on Arch):
    - `/etc/modules-load.d/uinput.conf`:
        ```
        uinput
        ```
- Run `sway-alttab` as your user, sudo is not needed
