# Clean-system installation of the GPUI Kit front end, Ubuntu 24.04 and Arch

The check of [2026-11-11](2026-11-11-clean-install.md) was repeated for the packages CI
built after the cutover: two throwaway virtual machines, each booted from the
distribution's official cloud image with nothing added, the archive and the AppImage
installed in them, the client started in a window on a compositor without a seat, and the
probe used against the lab with no internet. It is the first time the new client ran
outside the development workstation and the first time it drew into a window of a
compositor at all. No person saw or used that window: see the last section.

Every observation below was made with the binaries of the CI packages, in a virtual
machine. Nothing here was built on the workstation.

## What was run

- Packages: `xenadmin-rs-0.20.0-linux-x86_64.tar.gz` and `.AppImage` from
  [CI run 44436396899](https://github.com/z23/xenadmin-rs/actions/runs/44436396899) for
  commit `e808a2f` (the merge that completed the cutover), built on Ubuntu 24.04 without
  the `scenes` feature. The branch head when the check was made, `02bfe4e`, differs from
  that commit in documents, the changelog and one test file only. The client in the
  AppImage is the archive's, byte for byte.
- Machines: QEMU with KVM, four virtual CPUs, 4 GiB, no GPU, no display, user-mode
  networking, one at a time.
  - Ubuntu 24.04.5 LTS from `noble-server-cloudimg-amd64.img`: kernel 6.8.0-147, glibc
    2.39, Mesa 25.2.8 with LLVM 20. Packages were installed onto the image as it is; it was
    not upgraded.
  - Arch Linux from `Arch-Linux-x86_64-cloudimg.qcow2`, brought up to date with
    `pacman -Syu` on 2026-11-19 before anything was installed (a rolling system is not
    upgraded in part): glibc 2.44, Mesa 26.2.7, libwayland 1.26.0, Python 3.14.10. The
    kernel that ran stayed the image's, 7.2.11.
- The window: the earlier check used Weston's headless backend. The new client does not
  start there, because that backend offers no seat (below). The compositor was Sway with
  the wlroots headless backend and the pixman renderer (Sway 1.9 on Ubuntu, 1.12 on Arch),
  pictures were taken with `grim`, and every picture was looked at. Rendering was Mesa's
  software Vulkan driver (lavapipe), the only Vulkan driver present.
- The compositor's packages bring libraries of their own (Mesa's EGL and GL among them),
  and the client uses EGL when it finds it. So that the window says something about the
  documented packages, the client ran in a private mount namespace in which every shared
  object of a package installed after the documented list was covered by an empty file:
  258 files on Ubuntu, 91 on Arch. The same device took the Vulkan driver, the Vulkan
  loader, the fonts and fontconfig away in the cases that say so.
- "Offline" means the default route removed and IPv6 disabled, with only a route to the
  lab's subnet left; requests to a public address, by number and by name, then failed to
  connect (`curl` exit status 7).
- "First frame" is the time from the client's first protocol message to the first buffer it
  attached to its surface, read from libwayland's protocol trace. Neither the client nor the
  compositor reports a frame time.

## Results

| Check | Ubuntu 24.04 | Arch |
| --- | --- | --- |
| The packages verify against their checksum files inside the machine | Yes | Yes |
| On the bare image, the probe has every library and prints its help | Yes | Yes |
| On the bare image, libraries the client lacks | `libxkbcommon-x11.so.0` | `libxcb.so.1`, `libxkbcommon.so.0`, `libxkbcommon-x11.so.0` |
| On the bare image, `--version` answers | No: the loader's message, status 127 | No: the same |
| With the documented packages, `--version` answers from the archive and from the installed launcher | Yes, `xenadmin-rs 0.20.0` | Yes |
| The archive's installer puts the release under `~/.local` and reports it active | Yes | Yes |
| With the documented packages, the installed client opens its window | Yes | **No**: it panics for want of a font. Yes once `ttf-dejavu` is added |
| The window at 1440 by 900 in the dark theme: menu bar, toolbar, navigation pane with the synthetic pool, tabs, properties, status bar, readable text | Yes, looked at; first frame 137 ms | Yes, looked at; first frame 347 ms on the first start |
| The same in the light theme (`--theme light`) | Yes; 142 ms | Yes; 133 ms |
| At 800 by 600 (`--size 800x600`): the navigation pane goes behind its toggle and nothing is cut off | Yes; 133 ms | Yes; 116 ms |
| The AppImage run directly | Yes: `fuse3` and `/dev/fuse` are on the image | No: the runtime says no `fusermount` was found and names `--appimage-extract`, status 127 |
| The AppImage through `--appimage-extract-and-run`: `--version`, `--help`, `--probe --help`, and the window | Yes; the window is the archive's, pixel for pixel | Yes; the same |
| The third-party notices are in the archive and in the AppImage | Yes | Yes |
| Offline, the installed probe logs in through the member with each endpoint pinned to its own certificate, follows the redirect and lists the pool | Yes | Yes |
| A wrong certificate pin is refused before any credential is sent | Yes, for the member's pin and for the coordinator's | Yes, both |
| Uninstall removes the launchers and keeps the state | Yes | Yes |

The text was drawn with DejaVu Sans on both systems. On Ubuntu it comes from
`fonts-dejavu-core`, which is on the cloud image; the Arch image has no font at all.

The client took 200 to 230 MiB of resident memory ten seconds after its start on Ubuntu.

### The packages that were installed

The documented lists, as they stood before this check:

- Ubuntu: `libvulkan1 mesa-vulkan-drivers libwayland-client0 libxcb1 libxkbcommon0
  libxkbcommon-x11-0 libfontconfig1`. Three were on the image already (`libxcb1`,
  `libxkbcommon0`, `libfontconfig1`); fourteen packages were new: `libllvm20 libvulkan1
  libwayland-client0 libx11-xcb1 libxcb-dri3-0 libxcb-present0 libxcb-randr0 libxcb-shm0
  libxcb-sync1 libxcb-xfixes0 libxcb-xkb1 libxkbcommon-x11-0 libxshmfence1
  mesa-vulkan-drivers`.
- Arch: `vulkan-icd-loader vulkan-swrast wayland libxcb libxkbcommon libxkbcommon-x11
  fontconfig`, which brought twenty-four packages: `default-cursors fontconfig freetype2
  libdisplay-info libdrm libpciaccess libpng libx11 libxau libxcb libxdmcp libxkbcommon
  libxkbcommon-x11 libxshmfence llvm-libs spirv-tools vulkan-icd-loader
  vulkan-mesa-implicit-layers vulkan-swrast wayland xcb-proto xcb-util-keysyms
  xkeyboard-config xorgproto`.

Python, which the installer needs, was on both images. For the window, `sway` and `grim`
were added afterwards (and, on Ubuntu, `weston` and `strace`); for the probe nothing.

### What was wrong in the documented lists

1. **A font was missing from them.** The client asks the toolkit for its interface font
   and, failing that, for one of these families by name: Helvetica, Segoe UI, Ubuntu,
   Adwaita Sans, Cantarell, Noto Sans, DejaVu Sans, Arial. With none of them installed it
   does not start (the words are below). The aliases of fontconfig are not followed: with
   FreeSans (`gnu-free-fonts`, which Sway brought) and Liberation Sans (`ttf-liberation`)
   installed (fontconfig answers a request for Arial with Liberation Sans), it still did
   not start; with `ttf-dejavu` it did. On Ubuntu the list worked only because the image has
   `fonts-dejavu-core`: with `/usr/share/fonts` covered, the client failed there in the
   same way.
2. **fontconfig is not needed.** The client is not linked against it and does not load it.
   With the library covered (both systems), and on Arch `/etc/fonts` and
   `/usr/share/fontconfig` as well, the window was the same picture, pixel for pixel. The
   client reads fontconfig's configuration file itself when there is one.

The lists are corrected in [the installation guide](../INSTALLATION.md),
[the running guide](../RUNNING.md) and `packaging/runtime-sources.md`: the font package
replaces fontconfig (`fonts-dejavu-core` on Ubuntu, `ttf-dejavu` on Arch). The corrected
Arch list was not installed on its own on an untouched image: what was shown is the
documented list plus `ttf-dejavu`, with fontconfig and the compositor's libraries covered.

### What the client says when something is missing

A plain message was wanted in each case. These are the exact words; "panic" means the
Rust panic text with exit status 101, of which the second line is quoted.

| Situation | Seen on | What happens |
| --- | --- | --- |
| A link-time library is missing (bare image), any argument, `--version` included | both | The loader: `error while loading shared libraries: libxkbcommon-x11.so.0: cannot open shared object file: No such file or directory` (on Arch it names `libxcb.so.1`), status 127. Plain, but `--version` cannot answer |
| `--snap` in the packaged build | both | `xenadmin-rs: this build cannot draw a scene to a file: it was made without the `scenes` feature. Use the xenadmin-scenes tool.`, status 1. As intended |
| Neither `WAYLAND_DISPLAY` nor `DISPLAY` is set | both | **Defect.** Nothing is printed and the process runs until it is stopped, with no window: the toolkit starts headless. Without `--demo` it also takes the lock of the state directory |
| `WAYLAND_DISPLAY` names a socket that does not exist | both | **Defect**, a panic: `failed to connect to Wayland compositor at /run/user/1000/wayland-none: No such file or directory (os error 2)` |
| `DISPLAY` names a server that does not exist | both | **Defect**, a panic: `failed to connect to X server ":99": Unknown connection error` |
| No font of the families above | both | **Defect**, a panic: `failed to resolve font '.SystemUIFont' or any of the fallbacks: .ZedMono, .ZedSans, Helvetica, Segoe UI, Ubuntu, Adwaita Sans, Cantarell, Noto Sans, DejaVu Sans, Arial` |
| No Vulkan driver (the loader finds no driver manifest), or no Vulkan loader, and no EGL | both | **Defect.** `XenAdmin RS: The main window did not open: Failed to create surface: Failed to create surface for any enabled backend: {}`, and then the process does not end: 25 and 30 seconds later it was still there, sleeping, with no window, until it was stopped. The message does not name Vulkan |
| No Vulkan driver, but Mesa's EGL and GL present | Ubuntu | The window opens: the toolkit falls back to OpenGL. Mesa prints warnings |
| A compositor without a seat (Weston's headless backend, 15.0.1) | Arch | A panic: `Wayland compositor does not provide wl_seat`. The same was seen with Weston 13 on Ubuntu, where the words were not kept. No desktop compositor is without a seat; this is why Sway was used |

All but the first two are in the toolkit's start-up path, and none of them is changed in
the packages that were checked. The client's own start was changed afterwards: see "What
was changed in the client" below.

Smaller things seen: the AppImage started with `--appimage-extract-and-run` leaves its
`/tmp/appimage_extracted_*` directory behind when the client is ended by a signal;
uninstall leaves an empty `~/.local/lib/xenadmin-rs` directory; `~/.local/bin` is not on
the search path of Arch's image, as the installation guide warns; without `--theme` the
preview was light on these systems, which have no desktop portal to say otherwise.

### The notices

The archive carries `LICENSE` (the client's own licence, MIT) and `third-party/`:
`README.txt`, `dependencies.json` with 695 packages, one directory for each of the 695,
and `licenses/` with the standard texts `Apache-2.0.txt`, `MIT.txt` and `Zlib.txt` for the
packages that supply no notice of their own (the README names them). The AppImage carries
the same tree under `usr/`, identical file for file, and adds
`usr/third-party/appimage-runtime` with the runtime's licences and sources. No shared
library is in either package.

### The probe, offline

The lab's hosts were read only. The password was copied into the machine as a file of mode
0600, given to the probe on standard input and removed afterwards. On the day the member
was the second host and the coordinator the first. Asked alone with its own pin, the
coordinator answered; the member, asked alone, failed on the coordinator's certificate
(`TLS verification failed for https://…/. Verify this host's certificate and add its
endpoint-specific fingerprint`), which shows the redirect; with both pins it listed the
pool (2 hosts, 4 guests, 9 storage repositories, 8 networks). With one digit of either pin
changed, and a password that is not the lab's, the probe failed with the same TLS message
and status 1. The failure is the verification of the certificate, which comes before any
request is written; the traffic itself was not captured. On
Ubuntu one login with a wrong password and right pins was also made, to see the other
error (`SESSION_AUTHENTICATION_FAILED`).

### What was changed in the client

After the check, `crates/xenadmin-desktop/src/app.rs` was changed so that these starts end
with one plain line and status 1:

- no display named: `xenadmin-rs: no display: neither WAYLAND_DISPLAY nor DISPLAY is set.
  Start the client from a Wayland or X11 session.`, said before the state directory is
  claimed;
- a display that cannot be reached, a compositor without a seat, no font: the toolkit's
  panic is caught while the main window is not yet open and said as `xenadmin-rs: the
  display could not be reached: …`, `xenadmin-rs: the display cannot be used: …` or
  `xenadmin-rs: no usable font is installed. The interface needs one of these families:
  …`. With `RUST_BACKTRACE` set the panic is reported as before;
- no graphics backend: a line about the Vulkan driver is added to the toolkit's words, and
  the process now ends. It stayed because the toolkit forgets a quit that is asked for
  before its event loop runs; the quit is now asked for from a task.

The wording and the test for a named display are held by two unit tests of that file. That
the process ends in each case is not held by any test: it was seen once, with a debug
build of the workstation, in the Arch machine (no display, the two displays that cannot
be reached, no font, no Vulkan driver, no seat, and an ordinary start that still opened
its window under Sway; the case without a Vulkan loader was not repeated). That binary is not a package: CI
has not built this change, and it was not tried on Ubuntu.

## An X session without Wayland (Ubuntu)

The Ubuntu image was started again, untouched. Added to the documented packages:
`xserver-xorg-core` 21.1.12 with its `dummy` video driver (a virtual 1600 by 1000
display), Openbox 3.6.1, and `xdotool`, `xclip`, `wmctrl`, `x11-utils` and `x11-apps` to
drive and observe the session. No Wayland compositor existed in the machine;
`WAYLAND_DISPLAY` was unset and the session type was `tty`. The client was the CI archive,
unpacked, started with `--demo --theme dark`. X11 stays experimental (plan decision D7);
this records what happened.

| Check | Result |
| --- | --- |
| The window is mapped and managed: Openbox frames it with a title bar, and the client reports its name, class and process | Yes, 1440 by 900, listed 0.2 s after the start |
| The interface renders | Yes; it looks as it does under Wayland |
| A pointer press on a tab changes the tab | Yes (General to Memory) |
| Text typed into the search box of the navigation pane narrows the tree | Yes, capital included ("Gateway" left the six guests of that name) |
| Copy from the field reaches the X clipboard | Yes (Ctrl+A, Ctrl+C; `xclip` read "Gateway") |
| Text placed on the X clipboard pastes into the field | Yes (Ctrl+V of "Monitoring", then " 05" typed; the tree narrowed to that guest and the field read back "Monitoring 05") |
| A resize through the window manager lays the interface out again | Yes, at 900 by 640, with the navigation pane behind its toggle |
| Closing through the window manager ends the process cleanly | Yes: status 0 within 0.3 s |

The client's log held two warnings of Mesa's EGL about DRI3 and nothing else; the X
server's log held no error. The X server's packages bring Mesa's EGL and GL, which the
client loaded beside the Vulkan libraries: which of the two drew was not determined.

## What this does not show

- No person saw or used a window. Input was injected through the X test extension in the X
  session; under Sway there was no input at all. Keyboard layouts, the clipboard between
  applications in a Wayland session, file dialogs, the keyring and accessibility on a clean
  system are unverified.
- No hardware GPU: every frame came from the software driver, on a virtual output. GPU
  selection, presentation timing and a real monitor are unverified.
- No desktop environment: no launcher or icon was tried, no portal, no client-side
  decoration (Sway and Openbox both draw the title bar), no GNOME, no KDE.
- The window showed synthetic data only. The login and the inventory were read through the
  probe, not through the window, and no console was opened.
- Under Wayland the documented packages were isolated by covering the compositor's
  libraries, not by their absence from the disk. Under X this was not done.
- The Arch result is for that day's packages. The corrected lists were not installed alone
  on an untouched image.
- The packages are CI's build of `e808a2f`, not a release.

Logs and pictures are kept outside the repository.
