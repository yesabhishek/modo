//! Embedded assets: the component icon set plus the few Lucide icons Modo adds.
use std::borrow::Cow;

use gpui_kit::{AssetSource, Result, SharedString, assets::icon_assets};

icon_assets!(
    ExtraIcons,
    [
        Braces,
        ChevronsDownUp,
        ChevronsUpDown,
        Code,
        Columns2,
        Command,
        CornerDownLeft,
        Diff,
        Eye,
        File,
        FileCode,
        FileDiff,
        FileBraces,
        FileImage,
        FileTerminal,
        FileType,
        FilePlus,
        FileText,
        Files,
        FolderGit2,
        FolderOpen,
        GitBranch,
        GitCommitHorizontal,
        GitCompare,
        GitFork,
        Clock,
        Monitor,
        SlidersHorizontal,
        CircleDot,
        GitPullRequest,
        Image,
        Keyboard,
        Layers,
        ListTree,
        Lock,
        PanelLeft,
        Paintbrush,
        Rows2,
        Save,
        ScrollText,
        SquarePen,
        Terminal,
        TextSearch,
        Type,
        TextWrap,
        X,
    ]
);

/// Serves Modo's extra icons first, then the GPUI Kit component bundle.
pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        match path {
            "brand/mark.svg" => {
                return Ok(Some(Cow::Borrowed(include_bytes!(
                    "../assets/brand/mark.svg"
                ))));
            }
            "brand/mark-dark.svg" => {
                return Ok(Some(Cow::Borrowed(include_bytes!(
                    "../assets/brand/mark-dark.svg"
                ))));
            }
            _ => {}
        }
        if let Some(found) = ExtraIcons.load(path)? {
            return Ok(Some(found));
        }
        gpui_kit::assets::Assets.load(path)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut items = ExtraIcons.list(path)?;
        items.extend(gpui_kit::assets::Assets.list(path)?);
        items.sort();
        items.dedup();
        Ok(items)
    }
}
