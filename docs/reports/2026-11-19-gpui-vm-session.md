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

Two defects found here were fixed in this branch, and each fix was confirmed in the machine
with a binary built from the branch. Every row says which binary it was made with: **CI**
(the packaged AppImage) unless it says **branch build**.

Everything in this report is "verified in a virtual machine with injected input". None of it
is a person's use of the client on a desk; the last section says what that leaves open.

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

### C. Connections (gate G2, criterion 1)

Lab pool: coordinator h1 at 192.168.254.11, member h2 at 192.168.254.12, both with
self-signed certificates. Independent inspection of the certificates: Python's TLS on the
workstation and `openssl s_client | openssl x509 -fingerprint -sha256` in the machine gave the
same two SHA-256 values; h2 also prints its fingerprint on its own text console.

| Step | Result | Observed by |
| --- | --- | --- |
| Add Server, focus in the password field | endpoint `https://192.168.254.11` typed, user name `root` was prefilled; a press on Password: the field has the focus ring and the state `focused`, role `password text` | injected; frame 049 (looked at); AT-SPI |
| The password | typed from its file by injected keys; drawn as dots (frame 050). No frame shows it; the frames of the filled field show its length in dots. | injected; frames |
| First Connect | refused, as it must be: "TLS verification failed for https://192.168.254.11/. Verify this host's certificate and add its endpoint-specific fingerprint"; the connection is listed as "Connection failed" | frame 051 |
| Certificate review of h1 | the tree row's context menu has "Review this endpoint's certificate…" (frame 052). It opens the connect form with the certificate options; "Inspect certificate" shows validation "Certificate issuer is not trusted by the configured CA store", "No saved pin" and the presented SHA-256 (frame 055). The value copied by the form's copy button equals the independent inspection, compared by a script. "Confirm exact endpoint" typed, "I verified this SHA-256 through a trusted channel" ticked, "Trust certificate" pressed: "Certificate pin added to unsaved connection settings". | injected; frames 052 to 057; clipboard (`wl-paste`) |
| Connect with the pin | the password field was empty again and was typed a second time (frames 058, 059). Connected: "Connected directly • https://192.168.254.11/"; the tree shows the pool, both servers, the four guests under their servers, and the nine repositories; General shows "2 hosts · 4 guests · 39 templates · 0 snapshots · 9 storage repositories · 8 networks", inventory "Current" | frame 060 (looked at); AT-SPI |
| A change from outside | with `lab-guest-…` selected, another client set its description through XAPI. The General tab showed the new text within the four seconds waited, with no action in the client, and the selection and the tab stayed (frames 061, 062, looked at) | another client; frames; AT-SPI |
| Connections of the client | `ss -tn` in the machine: two connections to 192.168.254.11:443 and nothing else | `ss` |
| Close through the compositor | the process ends, no connection to the lab remains, `preferences.json` and `profiles.json` are written with mode 600 | compositor; `ss`; the files |
| No password on disk | a script that reads the password from its file searched every file of the state directory for it (raw, base64, hex, percent-encoded, UTF-16): **no**. Repeated at the end of the session: no. | script |
| Start again | the saved connection is listed on Home as "Disconnected"; eight seconds after the start the machine has no connection to the lab: nothing connects by itself | frame 070; AT-SPI; `ss` |
| Connect from the saved connection | "Connect…" in the row's context menu, password typed: connected with no certificate question (the pin was kept) | injected; frames 071, 072 |
| Disconnect, connect again | "Disconnect" in the pool's context menu: badge "Disconnected", inventory "Stale", status "No host connected", no connection in `ss` (frame 074). "Connect…" and the password: connected again (frames 076, 077) | injected; frames; `ss` |
| Certificate review of h2 | see finding 3 for how it has to be reached. The presented SHA-256 equals the independent inspection (and what h2 prints on its console); trusted the same way; connected | injected; frames 080 to 086; clipboard |
| Both pins typed up front (section E) | on an empty state directory, with the coordinator's fingerprint in "Certificate SHA-256" and `https://192.168.254.12=…` in "Additional host certificates": one Connect, no review, consoles on both servers work | injected; frame 094 |

Not observable: whether the server closed the client's session when the window was closed.
XAPI offers no list of sessions (`session.get_all_records` is refused as unknown).

### D. Console keyboard input (gate G3, criterion 3; route A of decision 0006)

The guest is `lab-guest-…` (Alpine, root without a password, US layout, on h1). What the guest
received is proved by the guest: before each test it starts recording its kernel's key events
(`/dev/input/event*`) and the test text is typed into `cat > /tmp/k`; afterwards the guest
sends the key events and the file's bytes to a listener on the lab bridge. Only the short
command that fetches these two scripts is typed. (The guest's network was down after its
last boot and was brought up by typed commands, which is itself keyboard input that worked.)

| Step | Result | Observed by |
| --- | --- | --- |
| The Console tab | the guest's console is drawn, fitted into the tab (frame 063, looked at) | frame |
| The keyboard by a pointer press | a press on the picture: `root`, Enter logs in; the guest's pointer device recorded the button presses | injected; frame 064; guest effect |
| A line with capitals, digits and every US punctuation sign | typed: ``The Quick Brown fox 0123456789 `~!@#$%^&*()-_=+[]{}\\|;:'",.<>/? zyZY``. The guest's file holds exactly these bytes. Its key events show each character's key, with Left Shift down and up as a key of its own around each shifted one. | injected; guest effect; frame 066 |
| The same physical keys under another layout | sway's layout set to `de` (where the text field of section F receives `yzßYö` for `zy-Z;`). The guest's file holds **the same bytes** as under `us`, and the same key codes. The guest received key codes, not the symbols of the local layout, which is what decision 0006 asks of route A. | injected; guest effect; frame 067 |
| Modifiers alone, both sides | Left and Right Shift, Ctrl, Alt and Super, pressed and released one after the other: the guest recorded each with its own code (Right Shift 54, Right Ctrl 97, Right Alt 100, Left and Right Meta 125 and 126), down then up | injected; guest effect |
| Modifiers held | Left Shift held over `a b c` gives `ABC` and one Shift down and up; Right Shift over `x` gives `X` | injected; guest effect |
| Application keys | Tab, Escape, F10, Alt+F and Ctrl+W reach the guest (the file holds a tab, ESC, the F10 sequence, ESC f, Ctrl+W). In the client nothing happened: the focus stayed on the console, no menu opened, nothing closed. | injected; guest effect; AT-SPI; frame 068 |
| Ctrl+Alt+F1 and Alt+Tab | only through the row's buttons, as the brief asks: the guest recorded Left Ctrl, Left Alt, F1 down and up in order, then Left Alt, Tab | injected (pointer); guest effect |
| A key held while the focus leaves | `a` held; one second later another window took the focus; the key stayed down at the machine for three more seconds. The guest recorded the release 1.2 s after the press, that is when the focus left. | injected; compositor; guest effect (timed) |
| A key held, focus kept | **CI: one press and no repeat.** `b` held for two seconds arrived as one `b` (down at 6.27 s, up at 8.29 s). Finding 2. **Branch build:** down, first repeat after 0.61 s, then one every 40 ms until the release; the guest's file holds 37 `b`. The key held while the focus leaves repeats until the focus leaves and no longer; a held Shift does not repeat. | injected; guest effect (timed) |

How it is known that the keys took route A and not the keystroke fallback, without reading a
log of key codes: the fallback derives a key code from the name of the symbol the toolkit
reports, by the US layout, and sends every modifier as the left key
(`crates/xenadmin-desktop/src/console/keys.rs`). Under the `de` layout it would have sent `y`
for the physical Z key, and it cannot send a right modifier at all. The guest received Z and
the five right-hand codes. The listener's second `wl_keyboard` is also visible in the protocol
log of a run without a password (a second `wl_seat` bind and `get_keyboard` 0.8 s after the
start).

### E. The recovery gate (gate G3, criterion 1; gate G5, criterion 2)

With the CI AppImage on an empty state directory.

| Step | Result | Observed by |
| --- | --- | --- |
| XO shut down | another client shut the appliance VM `XOA` down cleanly: power state Halted. XO does not answer: the workstation's probe fails to connect and `curl` from the machine times out. | another client; `curl` |
| The client connects | state directory removed, AppImage started, Add Server with both pins and the password (frames 094 to 096): connected | injected; frames |
| Locate the XO VM | `XOA` typed in the sidebar's search box: the tree shows the pool and `XOA` only (frame 097); a press selects it: General shows it halted (frame 098) | injected; frames; AT-SPI |
| Start it | "Start" in the toolbar. Another client then read: Running, on h1 (XAPI's own choice of server for a plain start). | injected; another client |
| Its console | the Console tab shows GRUB (frame 100, looked at), then the boot, then `Debian GNU/Linux 12 xoa tty1` and `xoa login:` (frame 102) | frames |
| Use the console | a press on the picture, then `P2-Keys_ok` typed: the prompt reads `xoa login: P2-Keys_ok` (frame 103 at actual size, looked at). Erased with ten Backspaces (frame 104). Nobody logged in; no password was typed. | injected; frames |
| Where the client's connections go | `ss -tn` in the machine during all of this: three connections to 192.168.254.11:443 and none to anything else. Nothing to the appliance's address. | `ss` |
| XO answers again | the workstation's probe: HTTPS 200; `curl` from the machine: 302 | another client; `curl` |
| Afterwards | another client moved the running appliance back to h2 (live migration, 6.4 s). The client's tree moved `XOA` from under h1 to under h2 by itself, the selection stayed on it, and its console reconnected: two connections to h1 and one to 192.168.254.12:443 (frames 105, 106). | another client; AT-SPI; frames; `ss` |

### F (on the lab). The host console and the rest

| Step | Result | Observed by |
| --- | --- | --- |
| The host's text console | h2 selected, Console tab: the server's text console is drawn, with its XCP-ng banner and the fingerprint it prints (frame 087, looked at). Nothing was typed into it; nobody logged in. | injected; frame |
| Save Screenshot of it | the portal's file chooser proposes `host-console.png`; saved with mode 600, a PNG of 640x384; the tab says "Console screenshot saved to /home/tester/host-console.png" | injected; frame 088; `stat` |
| Open in Separate Window | a second top-level window "… · XenAdmin RS console" (frame 089) | injected; compositor |
| Fullscreen of that window | sway reports `fullscreen_mode` 1 and the picture fills the display (frame 090, looked at); a second press: 0 | injected; compositor; frame |
| Closing that window through the compositor | only that window closes; the main window stays connected; the console's connection to h2 is gone | compositor; `ss` |
| Copy a live UUID | "Copy VM UUID" on XOA's General tab: `wl-paste` gives the UUID another client reads for that VM | injected; clipboard; another client |
| Search by a pasted UUID | Ctrl+V in the search box: the tree shows the pool, h2 and XOA only (frame 092) | injected; AT-SPI |
| A live grid | the pool's Search tab: sorted by name both ways, scrolled with the wheel, a row chosen by a press and moved by Down (frame 093) | injected; AT-SPI |

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
| Fullscreen | the button: sway reports `fullscreen_mode` 1 and no title bar; a second press: 0. In the protocol: `xdg_toplevel.set_fullscreen`, then `unset_fullscreen`. | injected; compositor; protocol; frames 028, 033 |
| Open in Separate Window | a second top-level window "Database · XenAdmin RS console" with the same app id, tiled beside the first | injected; compositor; frame 034 |
| A second start | the second process exits with status 0 and prints nothing; what the first window then shows is finding 1 | compositor; protocol; frame 046 |

### G. Accessibility

| Step | Result | Observed by |
| --- | --- | --- |
| Is the client on the bus | with `org.a11y.Status IsEnabled` false (this guest's default) it is not. With it true, the client registers (toolkit name `AccessKit`). | AT-SPI |
| The tree | the demo's first view: 93 nodes, 87 named. It is the scene tool's element tree of the same view (90 elements with a role, plus application, frame and menu bar), with the same counts per role and the same rectangles. Examples: `tree item 'Application 03'`, `page tab 'Memory'`, `entry 'Search inventory'`, `button 'Copy Pool UUID'`, `password text 'Password'`. | AT-SPI; the element tree |
| A screen reader | Orca 51.1 with its debug output in a file, while focus was moved by injected keys: 15 speech lines, among them "Demonstration pool - XenAdmin RS", "landmark Sidebar", "Application 03", "Infrastructure page tab", "Memory page tab", "Search inventory entry Search name, UUID or host". No sound device exists in the machine: this is what Orca would have spoken, taken from its log. | Orca's debug file |
| The password field | role `password text`; it offers no text interface, and a string typed into it was not found in any name, description, text or value on the bus | AT-SPI |

## Findings

### 1. A second start often showed nothing at all (fixed in this branch, confirmed in the machine)

What was done: the client runs; another window has the keyboard; the client is started a
second time from a shell.

What was seen with the CI package: the second process always exits with status 0 and prints
nothing, and the first client always asks the compositor for activation
(`xdg_activation_v1.get_activation_token`, then `activate`, in the protocol log). What the
compositor then does depended on where the pointer was:

- before any key or pointer press in the client: sway marks the window urgent (its window
  event `urgent`; the default policy of sway never gives focus on a request);
- after a press, with the pointer still over the client's window: urgent;
- after a press, with the pointer over another window: **nothing**. The window is neither
  focused nor urgent. Eleven of eleven second starts made in that state.

Cause: once a press has happened, the toolkit puts that press's serial and seat on the token
it requests (`gpui-pre-linux` 0.3.12, `linux/wayland/window.rs`, `activate`, and
`serial.rs`, `activation_context`). wlroots answers such a request with a token it will not
honour unless the window has the keyboard or the pointer. The token that the second start
received from its launcher was forwarded to the first client and not used: the toolkit's
`activate_window` takes none.

Fix (`crates/xenadmin-desktop/src/wayland_activate.rs`, used by the activation loop of
`app.rs`): on Wayland the first client also asks on the window's own display, as the console
key listener does for its keyboard. It passes the forwarded token to `activate` when there is
one, then requests a token without a serial and activates with it. Its two pure parts have
unit tests; the Wayland part can only be shown on a compositor.

Confirmed in the machine with a release binary **built from this branch's commit** (not the
CI package): after a press and with the pointer over another window, the second start marks
the first window urgent (two of two); with sway's policy set to `smart` and the second start
made by sway's launcher (`swaymsg exec`, which exports a token) the first window is focused,
where the CI package in the same case did nothing. Not shown: any other compositor.

### 2. A held key did not repeat in a guest (fixed in this branch, confirmed on the lab)

What was done: in the console of the Alpine guest, with the focus kept, the key `b` was held
for two seconds.

What was seen with the CI package: the guest's kernel recorded one press and, two seconds
later, one release; the text has one `b`. Backspace, the arrows and every other key behave the
same: holding does nothing more than one press.

Cause: route A takes its keys from a second `wl_keyboard`, and a `wl_keyboard` reports a held
key once. Repeating is the client's work, at the rate and after the delay the compositor
announces (`repeat_info`). The guest does not repeat by itself either: QEMU's keyboard has no
typematic repeat, a viewer is expected to send the repeats. The keystroke fallback repeats,
because the toolkit repeats its keystrokes; so the route the decision chose had lost what the
other had.

Fix (`crates/xenadmin-desktop/src/console/keys.rs`, `Repeater`, and
`wayland_keys.rs`, `SeatEvent::RepeatInfo`): the task that drains the listener every 8 ms
while a console has the keyboard also presses the key held last again, after the announced
delay and at the announced rate. Modifiers and lock keys do not repeat; the repeat stops when
the key goes up, when another key goes down, and when the keyboard leaves. Three unit tests
hold the timing rules.

Confirmed on the lab with a release binary **built from this branch's commit** `8d6ec1d`: see
the last row of section D. sway announces 25 repeats a second after 600 ms, and that is what
the guest recorded.

### 3. The certificate of a pool member: the client says what is wrong and offers no way to act

What was done: connected with only the coordinator's pin, then the Console tab of the member
h2 (and the same would meet anyone who opens the console of a guest that runs on it).

What was seen (CI): "Console disconnected: TLS verification failed for
https://192.168.254.12/. Verify this host's certificate and add its endpoint-specific
fingerprint. Reconnect when the guest or host is available.", automatic reconnect attempts
("attempt 3 of 5"), and no
control to review that certificate (frame 078). "Review this endpoint's certificate…" exists
only in the context menu of a connection that failed and is not live
(`can_review_certificate` in `crates/xenadmin-ui/src/areas/connections/home.rs`), and the
model refuses to change trust on a live connection. The way that worked: disconnect the whole
pool, "Connect…", open "Certificate options", type the member's address into "Certificate
endpoint", "Inspect certificate", confirm, "Trust certificate", type the password again,
connect. Nothing in the interface leads there.

For the first connection the same holds in a milder form: the failure is one line in the
status area, and the review is in the context menu of the tree row only; the General tab of
the failed connection has no action for it (frames 051, 052).

Not fixed here: where the action belongs is a decision about the command table and the model
of the connections area, not a slip. It is listed for the owner.

### 4. Smaller observations (CI package, not fixed)

- **The password is asked again after every step.** After the refused first attempt and
  again after the certificate review the password field is empty. Connecting to a pool of
  two self-signed servers through the reviews took four typings of the password; with both
  fingerprints typed up front it took one. This is the documented rule that a password lives
  in the session only, met at its least convenient.
- **No F10 and no Alt+letter for the menu bar.** The menus are reached by Tab (ten tab stops
  before the toolbar) and by the pointer.
- **Reasons cut off in menus.** In a context menu, an unavailable entry carries its reason
  after a dash, and the menu has a fixed width: "Install Guest Tools — No CD drive is
  available. Shut down the VM and add" ends there (frame 043).
- **"Type the exact endpoint above"** in the certificate review, while the field "Confirm
  exact endpoint" is below that sentence (frame 055).
- **Two spellings of the console's buttons.** The tab says "Actual Size", "Save
  Screenshot…", "Release Input"; the separate window says "Actual size", "Save screenshot…",
  "Release input" (frames 087, 090).
- **A stale tree keeps its last power states.** After Disconnect the pool is marked
  "Disconnected" and its inventory "Stale", and the guests in the tree still read "Running"
  (frame 074).
- **sway never focuses on request.** With the fix of finding 1 a second start marks the first
  window urgent under sway's default policy; it comes forward only under a policy that
  allows it. That is the compositor's choice.

## What the session says about the gates and about decision 0006

This report does not edit the gate reports. For whoever does:

- **Gate G2, criterion 1** ("connects to the lab pool with a pinned certificate and shows
  live inventory updates"): done in a real window with the packaged client, section C.
- **Gate G3, criterion 1, and gate G5, criterion 2** (the recovery gate with the packaged
  AppImage on an empty state directory): done, section E, in a virtual machine with injected
  input. The AppImage is the CI build of `e808a2f`, not a release artifact of a tag.
- **Gate G3, criterion 3** ("one driven desktop session (nested compositor, AT-SPI, physical
  key injection) covers connections, tables, search, console keyboard input and the host
  console"): each of the five was covered, sections C, D and F. The compositor was not nested:
  it ran on the display of a virtual machine. The keys were injected at the machine's
  keyboard. With the CI package the console's keyboard input had the defect of finding 2.
- **Decision 0006, the limits it lists.** "No guest recorded a key, and a second host layout
  was not applied": a guest recorded every key, and a second layout was applied; route A did
  what the decision expects of it. "Ctrl+Alt+F2 was not sent": Ctrl+Alt+F1 was sent through
  the row's button and arrived; no such chord was pressed at the keyboard. "GPUI did not
  deliver pointer motion or buttons": on sway the whole session was driven by pointer presses,
  and the guest recorded the button presses made on its console; pointer motion inside a guest
  was not examined. "Clipboard text was not offered": with the window focused, text was
  copied out and pasted in; without focus it was not tried. "Fullscreen was reported once and
  did not repeat. Exit was not observed": entered and left twice, on the main window and on a
  console window. "Screencopy never completed": fit and actual size were looked at in frames
  of the machine's display, not compared pixel by pixel. The hidden cursor and the guest's
  cursor were not examined. Route A has now run on sway 1.12 as well as on Hyprland 0.56.2,
  and on nothing else.
- **Plan decision D11** (accessibility): the client is on the accessibility bus when the bus
  is enabled, with a tree that matches the element tree, and Orca finds names and roles to
  speak, section G. Only the demo's views were read this way.

## Left undone

- The portable archive (`.tar.gz`) was checked against its checksum and not run; only the
  AppImage was.
- "Send Ctrl+Alt+Del" (it would have restarted the guest), "Send Clipboard to VM", "Type
  Clipboard as Keys" and "Copy VM Clipboard" were not pressed. Caps Lock and Num Lock were not
  tried. Pointer motion inside a guest was not examined.
- "Remember password in desktop keyring" and automatic connection at start: no keyring.
- Time to the first frame: one measurement.
- The two fixes ran on sway only, and the CI package does not have them until the branch is
  merged and built. (Merged, and built by CI on 2026-11-20; that package was run in this machine:
  [report](2026-11-20-gpui-merged-tree.md).)
- Orca was run on the demo, not on views of the lab.
- Whether the server closed the session when the window was closed could not be observed.
- Chords the compositor or the kernel may take for itself (Ctrl+Alt+F1 at the keyboard,
  Alt+Tab at the keyboard) were sent through the row's buttons only.

## The lab afterwards, and the checks of the branch

The lab was returned to its baseline and read back with the `lab_state` test and by another
client: both servers enabled, `XOA` running on h2, the two Alpine guests running on h1, the
desktop guest halted, no snapshot, HA off, and the description of `lab-guest-…` as it was
found. Inside that guest, until its next boot, its network interface holds the address it
was given for the reports, and it is logged out.

After the two fixes, at the code of commit `8d6ec1d`: `cargo fmt --all -- --check`,
`cargo clippy --workspace --all-targets --locked -- -D warnings`,
`cargo clippy -p xenadmin-desktop --all-targets --features scenes --locked -- -D warnings`,
`cargo test -p xenadmin-ui --locked` (977 passed) and
`cargo test -p xenadmin-desktop --features scenes --locked` (226 passed, the live tests
ignored) all passed. `./scripts/check.sh` was not run.

## What this does not show

- **No person.** Nobody looked at a screen or pressed a key. Every key and pointer event was
  injected at the machine's devices by a script, at a script's pace, and every picture was a
  capture of the machine's display read afterwards. Nothing here says how the client feels to
  use, whether text is comfortable to read, or whether an animation stutters.
- **No hardware GPU.** The client drew with Mesa's software Vulkan driver (llvmpipe) and sway
  drew with its software renderer. No GPU driver, no direct scan-out, no fractional or 2x
  scale, one output of 1440x900 at one refresh rate. Times measured here are of a software
  renderer in a virtual machine.
- **One compositor.** sway 1.12 (wlroots), tiling, with server-side decorations. Nothing was
  run on GNOME (Mutter, client-side decorations), on KDE (KWin) or on Hyprland, which is the
  operator's compositor and the one the spike of 2026-11-14 used.
- **A virtual seat.** One USB keyboard and one absolute pointer of QEMU, on a logind seat. No
  real keyboard with its own quirks, no touchpad, no touch, no tablet, no second keyboard, no
  input method (IME), no compose key, no key repeat by a person's hand.
- **X11 not at all.** The session has no X server and no Xwayland. The client's X11 path was
  not started once.
- **One distribution.** Arch Linux as updated on the day. The package was not run on Ubuntu
  here (its cloud kernel has no graphics driver for a display of the machine's own); that
  the package installs and starts there is the business of the clean-install run.
- **No keyring and no sound.** No Secret Service ran, so remembering a password was not
  tried. Orca's speech was read from its log, not heard.
- **The accessibility bus was on throughout**, because the driver finds controls through it.
  A session with the bus off was only used for the first start and the first frames.
