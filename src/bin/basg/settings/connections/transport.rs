use bas::service::Target;
use gpui_kit::{assets::IconName, component::Icon};

#[cfg(test)]
mod tests;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Hash)]
pub(super) enum Transport {
    #[default]
    Local,
    Tcp,
    Mtls,
    Unix,
    Pipe,
}

impl Transport {
    // Keep all variants for stored profiles, but only offer usable local transports.
    pub(super) const AVAILABLE: &[Self] = &[
        Self::Local,
        Self::Tcp,
        Self::Mtls,
        #[cfg(unix)]
        Self::Unix,
        #[cfg(windows)]
        Self::Pipe,
    ];

    pub(super) fn from_target(target: &Target) -> Self {
        match target {
            Target::Local => Self::Local,
            Target::Tcp(_) => Self::Tcp,
            Target::Mtls(_) => Self::Mtls,
            Target::Unix(_) => Self::Unix,
            Target::Pipe(_) => Self::Pipe,
        }
    }

    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Local => crate::i18n::tr!("connections-local"),
            Self::Tcp => "TCP",
            Self::Mtls => "mTLS",
            Self::Unix => "Unix socket",
            Self::Pipe => crate::i18n::tr!("connections-pipe"),
        }
    }

    pub(super) fn icon(self) -> Icon {
        match self {
            Self::Local => Icon::new(IconName::Monitor),
            Self::Tcp => Icon::new(IconName::Server),
            Self::Mtls => crate::connections::presentation::secure_icon(),
            Self::Unix | Self::Pipe => Icon::new(IconName::Cable),
        }
    }

    pub(super) fn placeholder(self) -> &'static str {
        match self {
            Self::Local => "",
            Self::Tcp | Self::Mtls => "hostname",
            Self::Unix => "/path/to/service.sock",
            Self::Pipe => "bas-work",
        }
    }

    pub(super) fn address_label(self) -> &'static str {
        match self {
            Self::Local | Self::Tcp | Self::Mtls => crate::i18n::tr!("connections-address"),
            Self::Unix => crate::i18n::tr!("connections-socket-path"),
            Self::Pipe => crate::i18n::tr!("connections-pipe-name"),
        }
    }

    pub(super) fn address(target: &Target) -> String {
        match target {
            Target::Local => String::new(),
            _ => crate::connections::presentation::endpoint(target),
        }
    }

    pub(super) fn target(self, value: &str) -> Result<Target, String> {
        let value = if matches!(self, Self::Tcp | Self::Mtls) {
            value
        } else {
            value.trim()
        };
        let (prefix, error) = match self {
            Self::Local => return Ok(Target::Local),
            Self::Tcp => ("tcp://", crate::i18n::tr!("connections-invalid-tcp")),
            Self::Mtls => ("mtls://", crate::i18n::tr!("connections-invalid-tcp")),
            Self::Unix => ("unix://", crate::i18n::tr!("connections-invalid-unix")),
            Self::Pipe => ("pipe://", crate::i18n::tr!("connections-invalid-pipe")),
        };
        let target: Target = format!("{prefix}{}", value.strip_prefix(prefix).unwrap_or(value))
            .parse()
            .map_err(|_| error.to_owned())?;
        Ok(target)
    }
}
