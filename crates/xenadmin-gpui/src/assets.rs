//! The asset source of the application: the Kit's default icons and the
//! icons of the Lucide catalogue the shell uses beyond them.
//!
//! An icon whose SVG the asset source does not serve draws nothing, without
//! an error. Every catalogue icon named in [`crate::shell::icons`] must be
//! listed here; a test checks it.

use std::borrow::Cow;

use gpui_kit::assets::{Assets, icon_assets};

use crate::kit::{AssetSource, SharedString};

icon_assets!(
    ShellIcons,
    [
        // Objects.
        Boxes,
        Server,
        Monitor,
        FileStack,
        Database,
        Camera,
        Layers,
        Tag,
        Shapes,
        Unplug,
        // Navigation views.
        ListTree,
        FolderTree, // Toolbar.
        ServerPlus,
        DatabasePlus,
        SquarePlus,
        Power,
        PowerOff,
        StepForward,
        Zap,
    ]
);

/// The Kit's assets with the shell's icons added.
#[derive(Clone, Copy, Debug, Default)]
pub struct AppAssets;

impl AssetSource for AppAssets {
    fn load(&self, path: &str) -> gpui_kit::Result<Option<Cow<'static, [u8]>>> {
        if let Some(bytes) = ShellIcons.load(path)? {
            return Ok(Some(bytes));
        }
        Assets.load(path)
    }

    fn list(&self, path: &str) -> gpui_kit::Result<Vec<SharedString>> {
        let mut all = Assets.list(path)?;
        all.extend(ShellIcons.list(path)?);
        all.sort();
        all.dedup();
        Ok(all)
    }
}
