//! The outcome of a memory write attempt.

use std::fmt;

/// The status of one memory add attempt.
///
/// This is the native counterpart of
/// `kala_agent.kernel.memory.MemoryStatus`, a `StrEnum` whose members are
/// `"added"`, `"durable"`, `"duplicate"` and `"conflict"`. The Python module
/// never produces `durable`; the member exists so the enumeration is
/// complete, and it is accepted by
/// [`crate::MemoryAddResult::accepted`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MemoryStatus {
    /// The record entered working memory.
    Added,
    /// The record is durable. Never produced by the manager; part of the
    /// enumeration for completeness.
    Durable,
    /// An equivalent record already existed.
    Duplicate,
    /// A different record already holds the same source.
    Conflict,
}

impl MemoryStatus {
    /// The Python enumeration value.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Added => "added",
            Self::Durable => "durable",
            Self::Duplicate => "duplicate",
            Self::Conflict => "conflict",
        }
    }

    /// Whether the attempt was accepted: added, durable or duplicate.
    pub const fn is_accepted(self) -> bool {
        matches!(self, Self::Added | Self::Durable | Self::Duplicate)
    }

    /// Parse a Python enumeration value.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "added" => Some(Self::Added),
            "durable" => Some(Self::Durable),
            "duplicate" => Some(Self::Duplicate),
            "conflict" => Some(Self::Conflict),
            _ => None,
        }
    }
}

impl fmt::Display for MemoryStatus {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}
