# Clean-system installation, Ubuntu 24.04 and Arch

Two throwaway virtual machines, each booted from the distribution's official cloud image
with nothing added, were used to install the packages CI built and to start the client with
no internet. This is the first installation evidence outside the development workstation.
It covers installation, software rendering, an offline start and a direct read-only
connection to the lab; it does not cover a desktop session a person uses.

## What was run

- Packages: `xenadmin-rs-0.18.0-linux-x86_64.tar.gz` and `.AppImage` from
  [CI run 42936800292](https://github.com/z23/xenadmin-rs/actions/runs/42936800292) for
  commit `c0c2b91`, built on Ubuntu 24.04. Both verified against their checksum files
  inside each machine.
- Machines: QEMU with KVM, four virtual CPUs, 4 GiB, no GPU, user-mode networking.
  - Ubuntu 24.04.5 LTS from `noble-server-cloudimg-amd64.img`, kernel 6.8.0, glibc 2.39.
  - Arch Linux from `Arch-Linux-x86_64-cloudimg.qcow2` as published that day, kernel
    7.2.11, glibc 2.44.
  Both images were checked against the publisher's SHA-256 before use.
- "Offline" means the default routes removed and IPv6 disabled, with only a route to the
  lab's isolated subnet left; requests to a public address then failed to connect.

## Results

| Check | Ubuntu 24.04 | Arch |
| --- | --- | --- |
| On the bare image, the binaries load (no missing link-time library) and the probe prints its help | Yes | Yes |
| On the bare image, a render fails with a plain message that no Vulkan driver could be loaded | Yes | Yes |
| After installing only the documented runtime packages, the archive's installer puts the release under `~/.local` and reports it active | Yes | Yes |
| The installed desktop renders a scene with the software Vulkan driver (the only kind present) | Yes, first frame 91 ms | Yes, first frame 83 ms |
| The AppImage runs its probe and renders a scene through `--appimage-extract-and-run` | Yes | Yes |
| Offline, the installed probe logs in through a lab member with each endpoint pinned to its own certificate, follows the redirect and lists the pool | Yes | Yes |
| A wrong certificate pin is refused before any credential is sent | Yes | not repeated |
| Offline, the installed desktop opens its window in a Wayland compositor (Weston 13, headless, software rendering) | Yes, captured | not run |
| Uninstall removes the launchers and keeps the state | Yes | Yes |

The runtime packages installed were, on Ubuntu, `libvulkan1 mesa-vulkan-drivers
libwayland-client0 libxkbcommon0 libxkbcommon-x11-0 libfontconfig1`, and on Arch,
`vulkan-icd-loader vulkan-swrast wayland libxkbcommon libxkbcommon-x11 fontconfig`. Python,
which the installer needs, was already on both images. Neither image has FUSE 2, so the
AppImage was run by extraction, as the installation guide describes.

Renders of the same scene were the same size to the byte on both machines.

## What this does not show

- No person used the window: the compositor in the virtual machine had no seat. Input,
  clipboard, file dialogs and accessibility on a clean system are unverified.
- No X11 session, no hardware GPU and no desktop environment's own launcher or icon.
- The login and inventory were read through the probe, not through the desktop window.
- The Arch image is a rolling snapshot; the result is for that day's packages.
