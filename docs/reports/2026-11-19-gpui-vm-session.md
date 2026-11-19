# GPUI front end: a driven desktop session in a virtual machine

2026-11-19. The first run of the GPUI Kit client in a real Wayland session. It was made in a
virtual machine, because nothing may open on the operator's desktop while they are away. The
machine carries the whole session: a display adapter and input devices of its own, a compositor
on them, and a driver outside that reads the machine's display and presses the machine's keys.

This report says for every step what was done, what was seen and how it was observed. The words
are used strictly:

- **injected**: pressed at the machine's keyboard or pointer device through QEMU's monitor
  (QMP `input-send-event`). The event passes the guest kernel's input device, the compositor's
  seat and the Wayland protocol, like a person's key. Nothing was injected inside the guest.
- **frame**: read from a picture of the machine's display (QMP `screendump`), looked at.
- **compositor**: read from sway's own report (`swaymsg -t get_tree`, its window events).
- **protocol**: read from the client's Wayland protocol log (`WAYLAND_DEBUG=1`).
- **AT-SPI**: read from the accessibility bus in the machine (used to find where a control
  is, and to read state; never to act).
- **guest effect**: what a lab guest itself recorded and reported.

WORK IN PROGRESS: the sections below are filled as the steps are run. A step without a result
has not been run.

## The machine

| | |
| --- | --- |
| Hypervisor | QEMU 11.1.2 with KVM on the workstation, `-display none`, user-mode networking, SSH forwarded to 127.0.0.1:2233 only |
| Devices | 4 CPUs, 6 GiB; `virtio-vga` 1440x900 (mode-setting driver `virtio_gpu`, output `Virtual-1`); `qemu-xhci` with `usb-kbd` and `usb-tablet`; a QMP socket |
| Where injected input arrives | keys on the guest kernel's `QEMU QEMU USB Keyboard`; pointer motion and buttons on `VirtualPS/2 VMware VMMouse` (QEMU routes absolute pointer events there, not to the tablet). Measured by reading the event devices while injecting. |
| Guest | Arch Linux, official cloud image, updated on the day (`pacman -Syu`); kernel 7.3.4-arch1-2; glibc 2.44 |
| Session | autologin on tty1, a logind seat (`seat0`, DRM master on `card0`), sway 1.12 on the machine's display with `WLR_RENDERER=pixman`; no Xwayland (not installed) |
| Renderer of the client | Mesa 26.2.7 software Vulkan, `llvmpipe (LLVM 23.1.4, 256 bits)`, device type CPU |
| Packages | sway 1.12-4, seatd 0.9.3-1 (installed, not used), foot 1.28.0-2, mesa 26.2.7-1, vulkan-swrast 26.2.7-1, vulkan-icd-loader 1.4.363.0-1, wayland 1.26.0-1, libxkbcommon 1.13.2-1, libxcb 1.17.0-1, fontconfig 2.18.4-1, ttf-dejavu, noto-fonts 2026.11.01-1, wl-clipboard 2.3.0-1, xdg-desktop-portal 1.22.1-2, xdg-desktop-portal-gtk 1.15.3-2 (the file-chooser backend, set in `portals.conf`), at-spi2-core 2.62.1-1, orca 51.1-1, gtk3 3.24.52-2, openssl 4.1.0-1 |
| Keyring | none: no Secret Service runs in the session, so "Remember password in desktop keyring" was never used |

## The packages

TODO

## Method

TODO

## Results

TODO

## Findings

TODO

## What this does not show

TODO
