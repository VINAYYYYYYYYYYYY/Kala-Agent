//! Typed, bounded failures for the CAD surface.
//!
//! The Python pipeline raises `ValueError` subclasses with stable,
//! human-readable messages. The native types preserve those messages exactly
//! where the source defines them, while giving callers a machine-readable
//! `kind` so a future API/session layer can decide whether a refusal is a
//! contract problem, a budget problem, an unsupported operation, or a kernel
//! failure.

use std::fmt;

/// Why a construction script was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CompileErrorKind {
    /// The JSON did not satisfy the frozen `kala-contracts` construction model.
    Contract,
    /// A supported contract operation is outside the Studio compiler subset.
    Unsupported,
    /// The wall-clock compile budget was exceeded.
    Budget,
    /// A compiler invariant or a validated-script sequencing rule failed.
    Invariant,
    /// The native kernel failed or raised an exception.
    Kernel,
    /// The crate was built without a functioning native kernel.
    Unavailable,
}

/// A construction compile refusal with the exact source-visible message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompileError {
    kind: CompileErrorKind,
    message: String,
}

impl CompileError {
    pub(crate) fn new(kind: CompileErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    #[cfg(feature = "native")]
    pub(crate) fn unsupported(operation: &str) -> Self {
        Self::new(
            CompileErrorKind::Unsupported,
            format!("unsupported by the Studio OCP compiler v1: {operation}"),
        )
    }

    #[cfg(feature = "native")]
    pub(crate) fn unsupported_detail(message: impl Into<String>) -> Self {
        Self::new(CompileErrorKind::Unsupported, message)
    }

    #[cfg(feature = "native")]
    pub(crate) fn invariant(message: impl Into<String>) -> Self {
        Self::new(CompileErrorKind::Invariant, message)
    }

    pub(crate) fn contract(message: impl Into<String>) -> Self {
        Self::new(CompileErrorKind::Contract, message)
    }

    #[cfg(feature = "native")]
    pub(crate) fn budget(seconds: f64) -> Self {
        Self::new(
            CompileErrorKind::Budget,
            format!(
                "the construction script exceeded the {} s compile budget",
                format_general(seconds)
            ),
        )
    }

    #[cfg(feature = "native")]
    pub(crate) fn kernel(class_name: &str) -> Self {
        Self::new(
            CompileErrorKind::Kernel,
            format!("the geometry kernel could not complete the script: {class_name}"),
        )
    }

    #[allow(dead_code)]
    pub(crate) fn unavailable() -> Self {
        Self::new(
            CompileErrorKind::Unavailable,
            "the native OpenCascade kernel is unavailable in this build",
        )
    }

    /// The machine-readable refusal class.
    pub fn kind(&self) -> CompileErrorKind {
        self.kind
    }

    /// The exact message, without a trailing newline.
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for CompileError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for CompileError {}

/// The shape could not be measured by the kernel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MeasurementError {
    message: String,
}

impl MeasurementError {
    pub(crate) fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    /// The exact message.
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for MeasurementError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for MeasurementError {}

/// The shape could not be tessellated inside the requested limits.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MeshError {
    message: String,
}

impl MeshError {
    pub(crate) fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    /// The exact message.
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for MeshError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for MeshError {}

/// STEP export or import failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StepError {
    message: String,
}

impl StepError {
    pub(crate) fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    /// The exact message.
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for StepError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for StepError {}

/// Why an HLR drawing or section cut was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DrawingErrorKind {
    /// An argument is out of range or a view/plane name is unknown. The
    /// Python module raises `ValueError` for these.
    Argument,
    /// The shape is null or has no finite bounds. The Python module raises
    /// `DrawingError` for these.
    Shape,
    /// The native geometry kernel failed or raised an exception.
    Kernel,
    /// The crate was built without a functioning native kernel.
    Unavailable,
}

/// An HLR drawing or section refusal with the exact source-visible message.
///
/// The Python module distinguishes `ValueError` (bad arguments) from
/// `DrawingError` (a shape or kernel that cannot be drawn). [`kind`] preserves
/// that distinction so a caller can map it to the same HTTP status the Python
/// worker produced.
///
/// [`kind`]: DrawingError::kind
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DrawingError {
    kind: DrawingErrorKind,
    message: String,
}

impl DrawingError {
    pub(crate) fn new(kind: DrawingErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    #[cfg(any(feature = "native", test))]
    pub(crate) fn argument(message: impl Into<String>) -> Self {
        Self::new(DrawingErrorKind::Argument, message)
    }

    #[cfg(any(feature = "native", test))]
    pub(crate) fn shape(message: impl Into<String>) -> Self {
        Self::new(DrawingErrorKind::Shape, message)
    }

    #[cfg(feature = "native")]
    pub(crate) fn kernel(message: impl Into<String>) -> Self {
        Self::new(DrawingErrorKind::Kernel, message)
    }

    #[allow(dead_code)]
    pub(crate) fn unavailable() -> Self {
        Self::new(
            DrawingErrorKind::Unavailable,
            "the native OpenCascade kernel is unavailable in this build",
        )
    }

    /// The machine-readable refusal class.
    pub fn kind(&self) -> DrawingErrorKind {
        self.kind
    }

    /// The exact message, without a trailing newline.
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for DrawingError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for DrawingError {}

/// Why a view-only STEP import was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum StepImportErrorKind {
    /// The configured byte ceiling is not a valid integer limit.
    Config,
    /// The bytes are not a readable ISO-10303-21 exchange file.
    Malformed,
    /// The bytes exceed the configured upload ceiling.
    TooLarge,
    /// The imported shape cannot be tessellated inside the mesh budget even
    /// after the single coarser retry.
    TooComplex,
}

/// A view-only STEP import refusal with the exact Python-visible message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StepImportError {
    kind: StepImportErrorKind,
    message: String,
}

impl StepImportError {
    pub(crate) fn new(kind: StepImportErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    /// The refusal class.
    pub fn kind(&self) -> StepImportErrorKind {
        self.kind
    }

    /// The machine-readable refusal class (`invalid_step`, `payload_too_large`,
    /// `step_too_complex`, or `invalid_config` for a bad limit).
    pub fn code(&self) -> &'static str {
        match self.kind {
            StepImportErrorKind::Config => "invalid_config",
            StepImportErrorKind::Malformed => "invalid_step",
            StepImportErrorKind::TooLarge => "payload_too_large",
            StepImportErrorKind::TooComplex => "step_too_complex",
        }
    }

    /// The exact message, without a trailing newline.
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for StepImportError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for StepImportError {}

/// Invalid declared or measured bounds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BoundsError {
    message: String,
}

impl BoundsError {
    pub(crate) fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    /// The exact message.
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for BoundsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for BoundsError {}

/// A native kernel call that failed. Only the exception class name crosses the
/// FFI boundary; product messages are built in Rust.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KernelError {
    class_name: String,
}

impl KernelError {
    #[cfg(feature = "native")]
    pub(crate) fn new(class_name: impl Into<String>) -> Self {
        Self {
            class_name: class_name.into(),
        }
    }

    /// The bounded OpenCascade/std exception class name.
    pub fn class_name(&self) -> &str {
        &self.class_name
    }
}

impl fmt::Display for KernelError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.class_name)
    }
}

impl std::error::Error for KernelError {}

/// Python's `format(value, "g")` for the finite, non-negative budget values
/// this crate formats. The compile budget is the only user-visible use; the
/// implementation is deliberately local so the crate does not grow a
/// formatting dependency for one diagnostic.
#[cfg(feature = "native")]
pub(crate) fn format_general(value: f64) -> String {
    if !value.is_finite() {
        return format!("{value}");
    }
    if value == 0.0 {
        return if value.is_sign_negative() {
            "-0".to_owned()
        } else {
            "0".to_owned()
        };
    }
    let exponent = value.abs().log10().floor() as i32;
    if !(-4..6).contains(&exponent) {
        let mut text = format!("{value:.5e}");
        if let Some((mantissa, exponent_text)) = text.split_once('e') {
            let mantissa = mantissa.trim_end_matches('0').trim_end_matches('.');
            let power: i32 = exponent_text.parse().unwrap_or(0);
            text = format!("{mantissa}e{power:+03}");
        }
        text
    } else {
        let decimals = (5 - exponent).max(0) as usize;
        let text = format!("{value:.decimals$}");
        let trimmed = text.trim_end_matches('0').trim_end_matches('.');
        if trimmed.is_empty() || trimmed == "-" {
            "0".to_owned()
        } else {
            trimmed.to_owned()
        }
    }
}

#[cfg(all(test, feature = "native"))]
mod tests {
    use super::format_general;

    #[test]
    fn general_format_matches_python_examples() {
        assert_eq!(format_general(5.0), "5");
        assert_eq!(format_general(0.5), "0.5");
        assert_eq!(format_general(0.0), "0");
        assert_eq!(format_general(1e-7), "1e-07");
        assert_eq!(format_general(123_456_789.0), "1.23457e+08");
        assert_eq!(format_general(12.3456789), "12.3457");
    }
}
