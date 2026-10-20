//! Structured validation errors.
//!
//! The public models are immutable and private-fielded; callers learn why a
//! value was refused from this type. The message vocabulary follows Pydantic
//! where the Python contract exposes it, without claiming to reproduce every
//! Pydantic error string (the construction loaders do preserve their exact
//! cleaned first message, which is product-visible).

use std::fmt;

/// One validation failure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContractIssue {
    /// Dotted/indexed path to the failing field, for example `tags[2]`.
    pub location: String,
    /// Human-readable message. Pydantic's `"Value error, "` prefix is removed.
    pub message: String,
}

impl ContractIssue {
    /// Build an issue at a path.
    pub fn new(location: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            location: location.into(),
            message: message.into(),
        }
    }

    /// Prefix a nested issue with a parent path.
    pub fn under(mut self, parent: &str) -> Self {
        if self.location.is_empty() {
            self.location = parent.to_owned();
        } else if parent.is_empty() {
            // Keep the existing path.
        } else {
            self.location = format!("{parent}.{}", self.location);
        }
        self
    }
}

/// Validation failure for one contract model or scalar.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContractError {
    pub(crate) model: &'static str,
    pub(crate) issues: Vec<ContractIssue>,
}

impl fmt::Display for ContractError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.model, self.message())
    }
}

impl std::error::Error for ContractError {}

impl ContractError {
    /// A single failure at the model root.
    pub fn new(model: &'static str, message: impl Into<String>) -> Self {
        Self {
            model,
            issues: vec![ContractIssue::new("", message)],
        }
    }

    /// A single failure at a field path.
    pub fn at(
        model: &'static str,
        location: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            model,
            issues: vec![ContractIssue::new(location, message)],
        }
    }

    /// Build from an existing issue list.
    pub fn from_issues(
        model: &'static str,
        issues: impl IntoIterator<Item = ContractIssue>,
    ) -> Self {
        let issues: Vec<ContractIssue> = issues.into_iter().collect();
        if issues.is_empty() {
            Self::new(model, "the value was refused")
        } else {
            Self { model, issues }
        }
    }

    /// Name of the model or scalar being validated.
    pub fn model(&self) -> &'static str {
        self.model
    }

    /// Every collected issue, in validation order.
    pub fn issues(&self) -> &[ContractIssue] {
        &self.issues
    }

    /// The first issue's message.
    pub fn message(&self) -> &str {
        self.issues
            .first()
            .map(|issue| issue.message.as_str())
            .unwrap_or("the value was refused")
    }

    /// The Python construction loader's message selection: an unknown
    /// operation anywhere wins, otherwise the first message is used.
    pub fn construction_message(&self) -> String {
        for issue in &self.issues {
            if issue.message.contains("unknown operation") {
                return issue.message.clone();
            }
        }
        self.message().to_owned()
    }
}
