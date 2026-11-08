# Linux packaging and local installation

Implemented portable installation/upgrade/removal and x86-64 AppImage assembly. This is
implementation and local packaging evidence, not a clean-distribution or real desktop
qualification claim. The scope rows are P18-01/P18-03 in the parity ledger.

## Delivered behavior

The extracted archive contains a strict file manifest and a Python 3.11 installer.
Installation verifies exact paths, hashes, file modes, size bounds and architecture, then
copies to a version/hash release directory. One atomic symlink selects the release. Previous
versions remain available; explicit activation verifies the selected files first. Repeating
installation repairs missing owned launchers. Prefix subdirectory symlinks and unowned or
modified files are refused. Removal checks the entire registered installation before deleting
application files and preserves profiles, preferences, keyring entries and operation journals.

The launcher follows the [freedesktop executable quoting rules](https://specifications.freedesktop.org/desktop-entry/latest/exec-variables.html),
including spaces, shell metacharacters and literal percent signs. Unsupported executable-path
characters are refused. No shell evaluates arguments passed to the application or installer.
Activation switches an executable; it does not downgrade state schemas or close a live client.

AppImage assembly verifies a pinned official appimagetool 1.9.1 and a pinned type-2 runtime
before execution. The tool receives an explicit runtime, as supported by the
[official appimagetool interface](https://github.com/AppImage/appimagetool/tree/1.9.1#usage).
The runtime's full source/build recipes/patches, libfuse and squashfuse sources and bundled
library notices accompany the image. Its source commit is
[`fedfad51acc2860b2916a1eef32ba4a500ff31ef`](https://github.com/AppImage/type2-runtime/tree/fedfad51acc2860b2916a1eef32ba4a500ff31ef).
The `continuous` asset URL is mutable; its pinned SHA-256 must match, or building stops.

The AppImage routes normal arguments to the desktop, `--probe` to host-probe and `--install`
to the installer. Extract-and-run works without a FUSE mount. Installed files are copied
from the image, so no launcher depends on a temporary mount. GPU drivers and desktop shared
libraries remain system dependencies. A build's glibc minimum remains that of its build
system; an AppImage wrapper does not make an Arch build compatible with older Ubuntu.

Package generation takes executable paths from the current Cargo compiler-artifact output,
checks the native ELF architecture, includes notices and emits SHA-256 files
and refuses replacement of existing versioned outputs. Ubuntu 24.04 CI now builds archive
and AppImage artifacts after normal checks. Adding that job does not establish that a remote
CI run has passed.

## Checks performed

- Twelve Python fixtures passed: install/upgrade/select/remove/reinstall, state preservation,
  integrity/path/symlink rejection, interrupted registration, missing launcher repair,
  modified ownership refusal, desktop escaping, cached-tool tampering, AppRun argument routing and target-specific Cargo artifact selection.
- Built a local AppImage from the existing 0.8 binary payload, labeled
  `0.8.0-packaging-test`; this synthetic packaging artifact is not a new feature release.
- Ran the AppImage without FUSE through extract-and-run, rendered the connection scene at
  1440×900, installed into a temporary prefix containing a space, validated the installed
  desktop file, queried the active release, rendered the installed executable at 513×652,
  and removed its application files while retaining its isolated state directory.
- Shell syntax and Python byte compilation passed. All captures contain synthetic data.

The existing 0.8 release archive was retained unchanged. No workstation `~/.local` files,
production client state, production host or desktop credentials were modified. Clean Ubuntu,
physical Wayland/X11 input/clipboard/Orca, hardware variations and offline XO-down acceptance
remain open. See [installation instructions](../INSTALLATION.md) for commands and requirements.

Peer review found and corrected target-specific Cargo output selection and missing staging-directory fsyncs. The installer now persists files and nested directories before publishing a release.
