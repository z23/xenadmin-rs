# Install, upgrade and remove XenAdmin RS

The Linux portable archive and x86-64 AppImage contain the application, read-only probe,
desktop metadata and dependency notices. They connect directly to XCP-ng; no host agent
or Xen Orchestra service is required. Development builds still require the qualification
checks recorded in [the parity ledger](https://github.com/z23/xenadmin-rs/blob/main/docs/PARITY-IMPLEMENTATION.md).

## Runtime requirements

Use a Linux desktop with Vulkan (hardware or Mesa software), libwayland-client,
libxkbcommon, libxkbcommon-x11 and fontconfig. Python 3.11 or newer is required only for
the optional installer. Native file selection uses the desktop portal; stored passwords
use the session's Secret Service provider. A missing keyring leaves manual credentials
available. GPU drivers and desktop libraries are supplied by the operating system.

Ubuntu 24.04 is the build baseline in CI. A build made on a newer distribution can require
its newer glibc; wrapping that build in an AppImage does not lower its runtime requirements.
Clean Ubuntu, real Wayland/X11, Orca and multi-monitor qualification are separate gates.

## Portable archive

Verify the archive against its adjacent SHA-256 file, then extract it into a directory you
own. A checksum detects damage but does not authenticate a download; use a trusted release
source. Run `bin/xenadmin-rs` directly. To add a desktop launcher and PATH entries:

```sh
python3 install.py install
```

The default prefix is `~/.local`. Ensure `~/.local/bin` is on PATH. A different prefix is
accepted with `python3 install.py --prefix /path/to/prefix install`. No root access or
system service is required. The installer verifies all package file hashes and modes,
refuses to overwrite unrelated/modified files and does not follow symlinked installation
subdirectories. The package manifest is an integrity inventory, not a code signature.

## Upgrade and select a previous version

Close the client before changing its active version. Run `install.py install` from the new
extracted package with the same prefix. Versions are retained in
`PREFIX/lib/xenadmin-rs/releases`; one atomic `current` link selects the active version.
The same exact package can be installed again to repair missing launchers. To list and
select a retained release:

```sh
xenadmin-rs-install status
xenadmin-rs-install activate VERSION-MANIFEST_HASH
```

Selecting an older executable does not downgrade operation journals or preferences.
Only select a version whose release notes explicitly permit reading the current state
schema. Keep a state-directory backup and resolve active/uncertain operations before
attempting a version rollback. The installer preserves all application state and never
starts or closes a client automatically.

## AppImage

Mark the downloaded image executable and run it. FUSE mounting is optional; environments
without working FUSE can run `./xenadmin-rs-VERSION-linux-x86_64.AppImage
--appimage-extract-and-run`. The image accepts the application's normal arguments.
`--probe` runs the bundled read-only host probe. For optional desktop integration:

```sh
./xenadmin-rs-VERSION-linux-x86_64.AppImage --appimage-extract-and-run --install install
```

This copies the verified portable payload out of the mounted/extracted image into the
managed prefix, so installed launchers do not depend on a transient mount. The image
contains the runtime's notices, source, build recipes and libfuse/squashfuse sources under
`usr/third-party/appimage-runtime`. Its official packaging tool and runtime inputs are
pinned by SHA-256; a changed upstream continuous runtime is refused until reviewed.

## Remove the application

```sh
xenadmin-rs-install uninstall
```

Removal checks every registered installed release and launcher before deleting owned
application files. Modified files are preserved and cause removal to stop with their path.
If the launcher is missing, use `python3 install.py --prefix PREFIX uninstall` from an
extracted package. Profiles, saved searches, history and keyring credentials are retained.
The state directory is `$XDG_STATE_HOME/xenadmin-rs` (normally
`~/.local/state/xenadmin-rs`), or the explicit `XENADMIN_STATE_DIR` override.

## Build packages

```sh
./scripts/package.sh
./scripts/package.sh --appimage
```

Both commands build the locked release workspace. The second additionally assembles an
AppImage from the same staged payload; it requires `desktop-file-validate` and downloads
hash-verified official inputs into the Cargo target directory's `package-tools` cache.
Neither command overwrites existing versioned outputs. Move previous development artifacts
aside or advance the package version before rebuilding. Checksums sit beside the outputs
under `artifacts/`. The AppImage uses an explicit pinned runtime rather than an implicit
latest-runtime download. CI uploads build artifacts; it does not publish a GitHub release.
