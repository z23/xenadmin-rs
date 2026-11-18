//! "Save screenshot…": the save prompt, the background PNG writer and its
//! result. Carried over from the Rune `console_export.rs`.
//!
//! The prompt goes through the file-chooser intent. Its reply does not say
//! why no path came back, so the Rune line "Cannot open the save dialog:
//! {error}" has no successor: a prompt that could not open is a cancel.

use std::io::{BufWriter, Write};
use std::path::Path;
use std::sync::mpsc;

use image::ImageEncoder;
use xenadmin_core::ConsoleFrame;

use super::session::Seat;
use crate::prelude::*;

#[derive(Default)]
pub struct Export {
    /// A prompt is open or a writer is running: no second screenshot yet.
    pub pending: bool,
    result: Option<mpsc::Receiver<Result<String, String>>>,
}

impl Export {
    /// A writer was started and has not reported yet.
    pub(super) fn writing(&self) -> bool {
        self.result.is_some()
    }

    /// The writer's outcome as a status line, once it is there.
    pub(super) fn poll(&mut self) -> Option<String> {
        let receiver = self.result.as_ref()?;
        let result = match receiver.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => return None,
            Err(mpsc::TryRecvError::Disconnected) => {
                Err("The screenshot writer stopped before reporting an outcome.".into())
            }
        };
        *self = Self::default();
        Some(result.unwrap_or_else(|error| error))
    }
}

/// Asks where to save the frame that is on screen at this press. The frame
/// is captured now: what is written is what the person saw, whatever the
/// console shows by the time the prompt is answered.
pub(super) fn choose(m: &mut Model, seat: Seat) {
    let Some(s) = super::session_mut(m, seat) else {
        return;
    };
    if s.export.pending {
        return;
    }
    let Some(frame) = s
        .snapshot
        .as_ref()
        .and_then(|snapshot| snapshot.frame.clone())
    else {
        return;
    };
    s.release();
    let serial = s.serial;
    s.export.pending = true;
    let name = if s.target.host.is_some() {
        "host-console.png"
    } else {
        "vm-console.png"
    };
    m.choose_file(
        FileRequest::save("Save console screenshot", name, move |m, path| {
            let Some(s) = super::session_mut(m, seat) else {
                return;
            };
            if s.serial != serial {
                return;
            }
            let Some(path) = path else {
                s.export.pending = false;
                return;
            };
            let (sender, receiver) = mpsc::channel();
            let wake = s.waker.clone();
            let frame = frame.clone();
            let spawned = std::thread::Builder::new()
                .name("console-screenshot".into())
                .spawn(move || {
                    let result = write_png(&path, &frame)
                        .map(|()| format!("Console screenshot saved to {}", path.display()))
                        .map_err(|error| format!("Cannot save the console screenshot: {error}"));
                    let _ = sender.send(result);
                    if let Some(wake) = wake {
                        wake();
                    }
                });
            if spawned.is_ok() {
                s.export.result = Some(receiver);
                s.message = Some("Saving the captured console frame…".into());
            } else {
                s.export.pending = false;
                s.message = Some("Cannot start the screenshot writer.".into());
            }
        })
        .filter("PNG image", ["png"]),
    );
}

/// Writes the frame as a PNG that only the user can read, through a
/// temporary file in the destination directory, so a failure never leaves a
/// partial picture behind or replaces an existing file with one.
pub fn write_png(path: &Path, frame: &ConsoleFrame) -> Result<(), String> {
    use std::os::unix::fs::OpenOptionsExt;
    let pixels = usize::from(frame.width) * usize::from(frame.height);
    if pixels == 0 || pixels > 8_294_400 || frame.rgba.len() != pixels * 4 {
        return Err("invalid console frame".into());
    }
    if path
        .symlink_metadata()
        .is_ok_and(|metadata| !metadata.is_file())
    {
        return Err("choose a regular file destination".into());
    }
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let temporary = parent.join(format!(".xenadmin-screenshot-{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| {
        let file = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&temporary)
            .map_err(|e| e.to_string())?;
        let mut writer = BufWriter::new(file);
        image::codecs::png::PngEncoder::new(&mut writer)
            .write_image(
                &frame.rgba,
                u32::from(frame.width),
                u32::from(frame.height),
                image::ExtendedColorType::Rgba8,
            )
            .map_err(|e| e.to_string())?;
        writer.flush().map_err(|e| e.to_string())?;
        writer.get_ref().sync_all().map_err(|e| e.to_string())?;
        std::fs::rename(&temporary, path).map_err(|e| e.to_string())?;
        std::fs::File::open(parent)
            .and_then(|dir| dir.sync_all())
            .map_err(|e| e.to_string())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;

    /// A directory under the temporary directory, removed when dropped.
    pub(crate) struct Scratch(pub std::path::PathBuf);
    impl Scratch {
        pub(crate) fn new(prefix: &str) -> Self {
            let path = std::env::temp_dir().join(format!("{prefix}-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// Rune: `console_export.rs`, moved.
    #[test]
    fn screenshot_round_trip_is_private_and_invalid_frames_do_not_replace_files() {
        use std::os::unix::fs::PermissionsExt;
        let directory = Scratch::new("xenadmin-png");
        let path = directory.0.join("console.png");
        let rgba = vec![255, 0, 0, 255, 0, 255, 0, 255];
        let frame = ConsoleFrame {
            width: 2,
            height: 1,
            rgba: rgba.clone().into(),
        };
        write_png(&path, &frame).unwrap();
        assert_eq!(image::open(&path).unwrap().to_rgba8().as_raw(), &rgba);
        assert_eq!(path.metadata().unwrap().permissions().mode() & 0o777, 0o600);
        let before = std::fs::read(&path).unwrap();
        assert!(write_png(&path, &ConsoleFrame { width: 3, ..frame }).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), before);
        assert_eq!(std::fs::read_dir(&directory.0).unwrap().count(), 1);
    }
}
