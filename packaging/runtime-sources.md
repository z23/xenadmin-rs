# AppImage runtime sources and notices

This package embeds the unmodified x86-64 AppImage type-2 runtime from commit
`fedfad51acc2860b2916a1eef32ba4a500ff31ef`. `inputs.json` records the exact runtime,
packaging tool, source archive and notice SHA-256 hashes. The packaging tool itself
is used at build time and is not part of the distributed application.

The runtime's full source, build recipes and patches are in `runtime-source.tar.gz`.
`fuse-3.15.0.tar.xz` contains the libfuse source; `squashfuse-0.5.2.tar.gz` contains the
squashfuse source. The runtime source includes the libfuse patch actually applied by
upstream and its static-link command. Its `BUILD.md` and Dockerfile describe rebuilding
and relinking the runtime with a modified library. These sources may be extracted and
modified independently of the XenAdmin application. To replace the embedded runtime,
rebuild this application using `packaging/appimage.py` with your reviewed runtime and
updated hash; no signature or application check prohibits a modified runtime/library.

The other statically linked components are musl, mimalloc, zstd and zlib. Their notices
are included alongside the runtime, libfuse and squashfuse licenses. The upstream build
uses Alpine 3.21 and its development packages. See the pinned upstream source and
`inputs.json` for the originating repositories. The notices of the application's Rust
dependencies are in the parent `third-party` directory.

The image holds the application's two executables and no desktop or graphics library. The
client is linked against libxcb, libxkbcommon and libxkbcommon-x11 and does not start without
them, in a Wayland session either; in a Wayland session it loads libwayland-client; it draws
through a Vulkan loader (libvulkan) and a Vulkan driver, a hardware one or Mesa's software
one; it draws text with a font of the host, which must have one of the families the client
looks for by name (DejaVu Sans and Noto Sans among them). All of these are the host's:
none is in this image, and the image replaces none. The fontconfig library is not used.

The runtime source and libfuse sources are supplied to permit modification and relinking
under the included LGPL terms; application state and host credentials are unrelated to
the packaging runtime and are never stored in this image.
