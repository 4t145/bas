use std::borrow::Cow;

use gpui_kit::{AssetSource, SharedString};

pub(super) const OUTLINE: &str = "branding/ferris-outline.svg";
pub(super) const OUTLINE_LIGHT: &str = "branding/ferris-outline-light.svg";
pub(super) const DIMENSIONAL: &str = "branding/ferris-dimensional.svg";

struct BrandingAsset {
    path: &'static str,
    bytes: &'static [u8],
}

const BRANDING: &[BrandingAsset] = &[
    BrandingAsset {
        path: OUTLINE,
        bytes: include_bytes!("../../../assets/branding/ferris-outline.svg"),
    },
    BrandingAsset {
        path: OUTLINE_LIGHT,
        bytes: include_bytes!("../../../assets/branding/ferris-outline-light.svg"),
    },
    BrandingAsset {
        path: DIMENSIONAL,
        bytes: include_bytes!("../../../assets/branding/ferris-dimensional.svg"),
    },
    BrandingAsset {
        path: "branding/ferris-dimensional-32.svg",
        bytes: include_bytes!("../../../assets/branding/ferris-dimensional-32.svg"),
    },
];

gpui_kit::assets::icon_assets!(
    ExtraIcons,
    [
        RefreshCw,
        Pencil,
        Trash,
        Square,
        Folder,
        FolderPlus,
        Maximize2,
        Minimize2,
        Send,
        Lightbulb,
        Image,
        File,
        Archive,
        ArchiveRestore,
        Menu,
        Info,
        LogOut,
        Pin,
        PinOff,
        LoaderCircle,
        MessageCircle,
        CircleCheck,
        CircleAlert,
        CircleStop,
        CornerLeftUp,
        Wrench,
        Terminal,
        FileText,
        FilePen,
        Globe,
        ListTodo,
        Circle,
        Clock,
        X
    ]
);

gpui_kit::assets::icon_assets!(RegistryIcons, [Download, Scale, Bot]);

pub(super) struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> anyhow::Result<Option<Cow<'static, [u8]>>> {
        if let Some(asset) = BRANDING.iter().find(|asset| asset.path == path) {
            return Ok(Some(Cow::Borrowed(asset.bytes)));
        }
        if let Some(bytes) = ExtraIcons.load(path)? {
            return Ok(Some(bytes));
        }
        if let Some(bytes) = RegistryIcons.load(path)? {
            return Ok(Some(bytes));
        }
        gpui_kit::assets::Assets.load(path)
    }

    fn list(&self, path: &str) -> anyhow::Result<Vec<SharedString>> {
        let mut paths = gpui_kit::assets::Assets.list(path)?;
        paths.extend(ExtraIcons.list(path)?);
        paths.extend(RegistryIcons.list(path)?);
        paths.extend(
            BRANDING
                .iter()
                .filter(|asset| asset.path.starts_with(path))
                .map(|asset| asset.path.into()),
        );
        paths.sort();
        paths.dedup();
        Ok(paths)
    }
}

#[cfg(test)]
mod tests;
