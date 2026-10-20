//! The configured isolation profile.

/// The isolation profile a tool run is bound to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IsolationProfile {
    /// Process confinement only. It is **not** a filesystem jail.
    Portable,
    /// Linux namespace, mount and chroot isolation. It is advertised only
    /// after a probe verified every required control.
    StrictLinux,
}

impl IsolationProfile {
    /// The Python `StrEnum` value.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Portable => "portable",
            Self::StrictLinux => "strict_linux",
        }
    }

    /// Parse the Python `StrEnum` value.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "portable" => Some(Self::Portable),
            "strict_linux" => Some(Self::StrictLinux),
            _ => None,
        }
    }
}

impl std::str::FromStr for IsolationProfile {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value).ok_or_else(|| format!("unknown isolation profile: {value}"))
    }
}

impl std::fmt::Display for IsolationProfile {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}
