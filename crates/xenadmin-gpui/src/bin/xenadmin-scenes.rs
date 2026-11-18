//! Renders a scene of the client to a PNG and to an element tree, without
//! a display.
//!
//! ```text
//! xenadmin-scenes --list
//! xenadmin-scenes NAME [--size WxH] [--theme dark|light] [--scale 1|2]
//!                      [--snap FILE.png] [--tree FILE.json]
//! ```
//!
//! `--snap` needs a graphics adapter (a software one will do); `--tree`
//! alone does not. The exit status is 0 when everything asked for was
//! written, 2 for a wrong command line and 1 for anything else.

use std::path::PathBuf;
use std::process::ExitCode;

use xenadmin_gpui::gpui_kit::component::ThemeMode;
use xenadmin_gpui::scenes::{self, Options};

const USAGE: &str = "usage: xenadmin-scenes --list
       xenadmin-scenes NAME [--size WxH] [--theme dark|light] [--scale 1|2] [--snap FILE.png] [--tree FILE.json]";

struct Request {
    name: String,
    options: Options,
    snap: Option<PathBuf>,
    tree: Option<PathBuf>,
}

enum Command {
    List,
    Render(Request),
}

fn parse(args: &[String]) -> Result<Command, String> {
    if args.iter().any(|arg| arg == "--list") {
        return Ok(Command::List);
    }
    let mut name = None;
    let mut options = Options::default();
    let (mut snap, mut tree) = (None, None);
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        let mut value = |flag: &str| {
            args.next()
                .cloned()
                .ok_or_else(|| format!("{flag} needs a value"))
        };
        match arg.as_str() {
            "--size" => {
                let size = value("--size")?;
                let parsed = size
                    .split_once('x')
                    .and_then(|(width, height)| Some((width.parse().ok()?, height.parse().ok()?)));
                match parsed {
                    Some((width, height)) if width >= 200. && height >= 200. => {
                        options.width = width;
                        options.height = height;
                    }
                    _ => return Err(format!("--size takes WIDTHxHEIGHT, not `{size}`")),
                }
            }
            "--theme" => {
                options.mode = match value("--theme")?.as_str() {
                    "dark" => ThemeMode::Dark,
                    "light" => ThemeMode::Light,
                    other => return Err(format!("--theme takes dark or light, not `{other}`")),
                }
            }
            "--scale" => {
                options.scale = match value("--scale")?.as_str() {
                    "1" => 1.,
                    "2" => 2.,
                    other => return Err(format!("--scale takes 1 or 2, not `{other}`")),
                }
            }
            "--snap" => snap = Some(PathBuf::from(value("--snap")?)),
            "--tree" => tree = Some(PathBuf::from(value("--tree")?)),
            flag if flag.starts_with("--") => return Err(format!("unknown option `{flag}`")),
            scene if name.is_none() => name = Some(scene.to_owned()),
            extra => return Err(format!("unexpected argument `{extra}`")),
        }
    }
    let name = name.ok_or("no scene was named")?;
    if snap.is_none() && tree.is_none() {
        return Err("nothing to write: give --snap, --tree or both".into());
    }
    Ok(Command::Render(Request {
        name,
        options,
        snap,
        tree,
    }))
}

fn run(request: &Request) -> Result<(), String> {
    scenes::write(
        &request.name,
        &request.options,
        request.snap.as_deref(),
        request.tree.as_deref(),
    )
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match parse(&args) {
        Ok(Command::List) => {
            for name in scenes::names() {
                println!("{name}");
            }
            ExitCode::SUCCESS
        }
        Ok(Command::Render(request)) => match run(&request) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("xenadmin-scenes: {error}");
                ExitCode::from(1)
            }
        },
        Err(error) => {
            eprintln!("xenadmin-scenes: {error}\n{USAGE}");
            ExitCode::from(2)
        }
    }
}
