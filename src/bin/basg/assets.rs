use std::borrow::Cow;

use gpui_kit::{AssetSource, SharedString};

pub(super) const OUTLINE: &str = "branding/ferris-outline.svg";
pub(super) const OUTLINE_LIGHT: &str = "branding/ferris-outline-light.svg";
pub(super) const DIMENSIONAL: &str = "branding/ferris-dimensional.svg";
pub(super) const SECURE_CONNECTION: &str = "icons/server-lock.svg";

struct EmbeddedAsset {
    path: &'static str,
    bytes: &'static [u8],
}

const EMBEDDED: &[EmbeddedAsset] = &[
    EmbeddedAsset {
        path: OUTLINE,
        bytes: include_bytes!("../../../assets/branding/ferris-outline.svg"),
    },
    EmbeddedAsset {
        path: OUTLINE_LIGHT,
        bytes: include_bytes!("../../../assets/branding/ferris-outline-light.svg"),
    },
    EmbeddedAsset {
        path: DIMENSIONAL,
        bytes: include_bytes!("../../../assets/branding/ferris-dimensional.svg"),
    },
    EmbeddedAsset {
        path: "branding/ferris-dimensional-32.svg",
        bytes: include_bytes!("../../../assets/branding/ferris-dimensional-32.svg"),
    },
    EmbeddedAsset {
        path: SECURE_CONNECTION,
        bytes: include_bytes!("../../../assets/icons/server-lock.svg"),
    },
];

gpui_kit::assets::icon_assets!(
    ExtraIcons,
    [
        RefreshCw,
        ArrowLeft,
        RotateCcw,
        RotateCw,
        Minus,
        Pencil,
        Trash,
        Square,
        Folder,
        FolderPlus,
        Monitor,
        Server,
        Activity,
        Cable,
        Lock,
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
        LogIn,
        LogOut,
        Bookmark,
        Pin,
        PinOff,
        LoaderCircle,
        MessageCircle,
        MessageSquare,
        MessagesSquare,
        Play,
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
        ListClock,
        ListX,
        Circle,
        Clock,
        Volume2,
        Play,
        Pause,
        X
    ]
);

gpui_kit::assets::icon_assets!(RegistryIcons, [Download, CircleArrowUp, Scale, Bot]);

pub(super) struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> anyhow::Result<Option<Cow<'static, [u8]>>> {
        if let Some(asset) = EMBEDDED.iter().find(|asset| asset.path == path) {
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
            EMBEDDED
                .iter()
                .filter(|asset| asset.path.starts_with(path))
                .map(|asset| asset.path.into()),
        );
        paths.sort();
        paths.dedup();
        Ok(paths)
    }
}
