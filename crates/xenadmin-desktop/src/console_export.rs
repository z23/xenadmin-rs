use super::*;
use image::ImageEncoder;
use std::{
    io::{BufWriter, Write},
    path::Path,
    sync::mpsc,
};
use xenadmin_core::ConsoleFrame;

#[derive(Default)]
pub(super) struct Export {
    pub pending: bool,
    result: Option<mpsc::Receiver<Result<String, String>>>,
}

pub(super) fn moving_window(s: &mut State) {
    // An already-authorized writer can finish; an old window's pending file dialog cannot.
    if s.console_ui.export.pending && s.console_ui.export.result.is_none() {
        s.console_ui.export = Default::default();
    }
}
pub(super) fn choose(s: &mut State, tx: &mut Tx) {
    if s.console_ui.export.pending {
        return;
    }
    let Some(frame) = s
        .console_snapshot
        .as_ref()
        .and_then(|snapshot| snapshot.frame.clone())
    else {
        return;
    };
    release(s);
    let serial = s.console_ui.serial;
    s.console_ui.export.pending = true;
    tx.save_file(
        rune::FileDialogOptions {
            title: "Save console screenshot".into(),
            file_name: Some(
                if host(s).is_some() {
                    "host-console.png"
                } else {
                    "vm-console.png"
                }
                .into(),
            ),
            ..Default::default()
        }
        .filter("PNG image", &["png"]),
        move |s: &mut State, result, _| {
            if s.console_ui.serial != serial {
                return;
            }
            match result {
                Ok(Some(path)) => {
                    let (sender, receiver) = mpsc::channel();
                    let wake = s.waker.clone();
                    let spawned = std::thread::Builder::new()
                        .name("console-screenshot".into())
                        .spawn(move || {
                            let result = write_png(&path, &frame)
                                .map(|()| format!("Console screenshot saved to {}", path.display()))
                                .map_err(|error| {
                                    format!("Cannot save the console screenshot: {error}")
                                });
                            let _ = sender.send(result);
                            if let Some(wake) = wake {
                                wake();
                            }
                        });
                    if spawned.is_ok() {
                        s.console_ui.export.result = Some(receiver);
                        s.console_ui.message = Some("Saving the captured console frame…".into());
                    } else {
                        s.console_ui.export.pending = false;
                        s.console_ui.message = Some("Cannot start the screenshot writer.".into());
                    }
                }
                Ok(None) => s.console_ui.export.pending = false,
                Err(error) => {
                    s.console_ui.export.pending = false;
                    s.console_ui.message = Some(format!("Cannot open the save dialog: {error}"));
                }
            }
        },
    );
}

pub(crate) fn poll(s: &mut State) -> bool {
    let Some(receiver) = &s.console_ui.export.result else {
        return false;
    };
    let result = match receiver.try_recv() {
        Ok(result) => result,
        Err(mpsc::TryRecvError::Empty) => return false,
        Err(mpsc::TryRecvError::Disconnected) => {
            Err("The screenshot writer stopped before reporting an outcome.".into())
        }
    };
    s.console_ui.export = Export::default();
    s.console_ui.message = Some(result.unwrap_or_else(|error| error));
    true
}

fn write_png(path: &Path, frame: &ConsoleFrame) -> Result<(), String> {
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
mod tests {
    use super::*;

    #[test]
    fn chooser_captures_one_frame_and_cancel_or_console_switch_does_not_write() {
        let directory =
            std::env::temp_dir().join(format!("xenadmin-png-dialog-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&directory).unwrap();
        let mut state = crate::init();
        let red = vec![255, 0, 0, 255];
        state.console_snapshot = Some(std::sync::Arc::new(xenadmin_core::ConsoleSnapshot {
            state: ConsoleState::Connected,
            frame: Some(std::sync::Arc::new(ConsoleFrame {
                width: 1,
                height: 1,
                rgba: red.clone().into(),
            })),
            cursor: None,
            guest_clipboard: None,
            clipboard_utf8: false,
            extended_keys: false,
        }));
        let mut runtime = rune::Runtime::new(
            state,
            |_: &State, cx: &mut rune::Cx<State>| {
                Button::new("Save")
                    .on_press_tx(choose)
                    .build(&crate::theme::kit(cx.system_dark()))
                    .key("save")
            },
            (480.0, 640.0),
            rune::Env {
                system_dark: true,
                scale: 1.0,
            },
        );
        let pick = |runtime: &mut rune::Runtime<State>| {
            runtime.frame(0.0);
            let (x, y) = runtime.find("save").unwrap();
            runtime.pointer_down(x, y);
            runtime.pointer_up(x, y);
            runtime.frame(0.0);
            assert_eq!(runtime.file_dialog().unwrap().0, rune::FileDialogKind::Save);
        };
        pick(&mut runtime);
        runtime.resolve_file_dialog(None).unwrap();
        runtime.frame(0.0);
        assert!(!runtime.state.console_ui.export.pending);
        pick(&mut runtime);
        let path = directory.join("frame.png");
        std::sync::Arc::make_mut(runtime.state.console_snapshot.as_mut().unwrap()).frame =
            Some(std::sync::Arc::new(ConsoleFrame {
                width: 1,
                height: 1,
                rgba: vec![0, 255, 0, 255].into(),
            }));
        runtime.resolve_file_dialog(Some(path.clone())).unwrap();
        runtime.frame(0.0);
        for _ in 0..500 {
            if poll(&mut runtime.state) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        assert!(!runtime.state.console_ui.export.pending);
        assert_eq!(image::open(&path).unwrap().to_rgba8().as_raw(), &red);
        pick(&mut runtime);
        super::super::reconnecting(&mut runtime.state);
        let stale = directory.join("stale.png");
        runtime.resolve_file_dialog(Some(stale.clone())).unwrap();
        runtime.frame(0.0);
        assert!(!stale.exists());
        std::fs::remove_dir_all(directory).unwrap();
    }
    #[test]
    fn screenshot_round_trip_is_private_and_invalid_frames_do_not_replace_files() {
        use std::os::unix::fs::PermissionsExt;
        let directory = std::env::temp_dir().join(format!("xenadmin-png-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&directory).unwrap();
        let path = directory.join("console.png");
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
        assert_eq!(std::fs::read_dir(&directory).unwrap().count(), 1);
        std::fs::remove_dir_all(directory).unwrap();
    }
}
