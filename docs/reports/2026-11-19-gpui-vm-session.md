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

The portable archive and the AppImage built by CI (workflow `ci.yml`, artifact `linux-x86_64`)
in run 44436396899 for commit `e808a2f` (the merge that completed the cutover). Both files
matched their `.sha256` files on the workstation and again in the machine. A later successful
run exists for `02bfe4e`; the two commits between change tests and documents only
(`git diff --stat e808a2f 02bfe4e`), so the client is the same.

The AppImage is `xenadmin-rs-0.20.0-linux-x86_64.AppImage`; `--version` prints
`xenadmin-rs 0.20.0`. It was run as downloaded, mounted by its own runtime (the machine has
`/dev/fuse` and `fuse3`, no `fuse2`; nothing was added for it). `--snap` is refused with
"this build cannot draw a scene to a file: it was made without the `scenes` feature", exit 1.

Unless a row says otherwise, every observation below was made with this AppImage.

## Method

1. A launcher derived from the clean-install `vm.sh` starts the machine described above. It
   opens no window: the display adapter exists only for the guest.
2. In the guest, `tester` logs in on tty1 automatically and its shell starts sway on the
   machine's display. The client is started from an SSH session into that Wayland session
   (`WAYLAND_DISPLAY=wayland-1`, the session's D-Bus address), the way a terminal in the
   session would start it. SSH is used to start programs and to read logs, sway's state and
   the accessibility bus. It is never used to press a key or a button of the client.
3. A driver on the workstation speaks QMP: `screendump` for frames, `input-send-event` for
   keys (QEMU key codes, that is physical keys of a US keyboard) and for the pointer
   (absolute position, buttons, wheel).
4. Where a control is: the client's accessibility tree gives the window-relative rectangle of
   an element by its name and role, sway's tree gives the window's place, and the driver
   presses the pointer in the middle. The accessibility bus was therefore switched on for the
   whole session (`org.a11y.Status IsEnabled`), which is not the default of this guest.
5. The lab password is typed by the driver from its file as physical keys. It is in no
   command line, log or frame. The protocol log (`WAYLAND_DEBUG`) records every key code, so
   it was never on while the password was typed.
6. Frames and logs are kept outside the repository, under the lab's evidence directory
   (`evidence/gpui-vm/`). Frame numbers below are the numbers of the files there.

## Results

Each row: what was done, the result, how it was observed.

### A. The machine and the session

| Step | Result | Observed by |
| --- | --- | --- |
| sway on the machine's display | up on `Virtual-1` at 1440x900, DRM master on the virtio adapter | frame 001; compositor |
| A key from outside | text typed into a terminal (foot) arrived and was written to a file in the guest | injected; frame 002; the file |
| The pointer from outside | the pointer is drawn where it was sent, and presses act there | injected; frames |

### B. The client starts

| Step | Result | Observed by |
| --- | --- | --- |
| Start on an empty state directory | the main window is drawn: menu bar, toolbar, sidebar with the empty tree, Home tab | frames 003, 004 (looked at) |
| Time to the window | 1.7 s from the start of the process to sway's "new window" event (one measurement, software rendering, a warm page cache not ensured) | compositor |
| What it logs | only Mesa's lines about the missing GPU: `MESA-EGL: warning: failed to get driver name for fd -1`, `MESA: error: ZINK: failed to choose pdev`, `egl: failed to create dri2 screen`. Nothing from the client itself. | log |
| Renderer | llvmpipe, through the Vulkan loader | `vulkaninfo`; the protocol log's `mesa vk display queue` |
| `--demo` | the synthetic pool is shown; nothing is written to the state directory | frame 005 |
| Menu bar by pointer | a press on a menu opens it; a press on an entry runs it | injected; frame 006 |
| Menu bar by keys | Alt+F and F10 do **not** open the menu bar (frames 007, 008). Each menu is a tab stop: Tab to it, Enter opens it, the arrows move inside, Left and Right change menu, Enter runs the entry (frames 009 to 011, 047, 048) | injected; frames; AT-SPI (focused element) |
| Tree | a row is selected by a press and by the arrow keys | injected; frame 012 |
| A tab, the search box | the Storage tab is shown by a press; text typed with Shift arrives in the search box and filters the tree | injected; frames 013, 014 |
| Decorations | sway draws the title bar: the client asks for `zxdg_toplevel_decoration_v1.set_mode(server_side)` and sway confirms it | protocol; frames |
| Move, resize, minimum | floating, the window is dragged by sway's title bar; it asks for a minimum of 800x600 (`set_min_size`) and at 796x600 of content the sidebar folds and nothing overlaps | injected; frames 015 to 018; protocol |
| Close by the compositor | `swaymsg kill` (the compositor's close request): the process ends, the activation socket is removed, `preferences.json` is written | compositor; the files |

### F (without the lab). Tables, search, clipboard, file chooser, windows

Made on `--demo` and on the scene `console-frame`, so that they are known before the lab is used.

| Step | Result | Observed by |
| --- | --- | --- |
| Grid sorted | the Search tab's grid: a press on "Sort by Name" sorts ascending, a second press descending | injected; AT-SPI (row order); frame 039 |
| Grid scrolled with the wheel | five wheel steps move the rows (first visible row changes) | injected; AT-SPI; frame 040 |
| A row by pointer and by keys | a press selects the row; Down and Up move the selection and the grid follows | injected; AT-SPI (selected state); frame 041 |
| Context menu at the pointer | a right press on a row opens the object's menu with its left edge at the pointer; it opens upward when there is no room below. The reasons of unavailable entries are cut at the menu's right edge. | injected; frame 043 |
| Search | text typed in the sidebar's search box filters the tree; the Search tab lists results | injected; frames 014, 021 |
| Copy to the clipboard | "Copy Pool UUID" on the General tab: `wl-paste` in the machine prints the UUID shown | injected; `wl-paste` |
| Paste from the clipboard | text put there with `wl-copy` (with two non-ASCII letters) is pasted into the search box by Ctrl+V | injected; frame 036 |
| Text follows the layout | with sway's layout set to `de`, the physical keys Z Y - Shift+Z ; give `yzßYö` in the search box (`zy-Z;` under `us`). This is the control for the console test of section D. | injected; frames 037, 038 |
| Save Screenshot | the button opens the GTK file chooser of the desktop portal ("Save File", app id `xdg-desktop-portal-gtk`) with the name `vm-console.png`; Save writes the file with mode 600, a PNG of the console's 800x600 | injected; frame 031; `stat`, `file` |
| Fullscreen | the button: sway reports `fullscreen_mode` 1 and no title bar; a second press: 0. In the protocol: `xdg_toplevel.set_fullscreen`, then `unset_fullscreen`. | injected; compositor; protocol; frames 028, 029 |
| Open in Separate Window | a second top-level window "Database · XenAdmin RS console" with the same app id, tiled beside the first | injected; compositor; frame 034 |
| A second start | see finding 1 | compositor; protocol |

### G. Accessibility

| Step | Result | Observed by |
| --- | --- | --- |
| Is the client on the bus | with `org.a11y.Status IsEnabled` false (this guest's default) it is not. With it true, the client registers (toolkit name `AccessKit`). | AT-SPI |
| The tree | the demo's first view: 93 nodes, 87 named. It is the scene tool's element tree of the same view (90 elements with a role, plus application, frame and menu bar), with the same counts per role and the same rectangles. Examples: `tree item 'Application 03'`, `page tab 'Memory'`, `entry 'Search inventory'`, `button 'Copy Pool UUID'`, `password text 'Password'`. | AT-SPI; the element tree |
| A screen reader | Orca 51.1 with its debug output in a file, while focus was moved by injected keys: 15 speech lines, among them "Demonstration pool - XenAdmin RS", "landmark Sidebar", "Application 03", "Infrastructure page tab", "Memory page tab", "Search inventory entry Search name, UUID or host". No sound device exists in the machine: this is what Orca would have spoken, taken from its log. | Orca's debug file |
| The password field | role `password text`; it offers no text interface, and a string typed into it was not found in any name, description, text or value on the bus | AT-SPI |

## Findings

TODO (being filled)

## What this does not show

TODO
