# Native certificate selection and console image export

The connection editor now opens the desktop portal's file chooser for a custom CA file.
Manual path entry remains available when the portal is unavailable. A delayed chooser result
is tied to the editor that opened it; cancellation leaves the existing value untouched.

The console toolbar now offers **Save screenshot…**. It captures the frame at the moment
the control is pressed and asks for a PNG destination. Encoding and file writing run off
the UI thread. A private temporary file is synchronized and renamed into place, so invalid
frames and incomplete encoding cannot replace an existing image. Destination files use
mode `0600`. A console replacement before the chooser returns discards that answer.
The control is explicit; no screenshot is captured or uploaded automatically.

Three local tests exercise the CA chooser's cancellation/editor binding, PNG pixel round-trip
and private permissions, invalid-frame preservation, captured-frame identity and console
replacement. They use synthetic pixels and scripted native-dialog answers. Actual portal
interaction under Wayland/X11 remains a separate desktop qualification check. No private
screenshots or host information were added to this repository.
