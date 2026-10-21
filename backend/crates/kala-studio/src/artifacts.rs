//! The stored-artifact download surface, shared by every build.
//!
//! One design can store four kinds of artifact: the canonical construction
//! `script`, the provider's `step`, the provider's native document (`fcstd`
//! for FreeCAD) and a display `stl` previously written by the export route.
//! The download route serves exactly those bytes, under exactly those kinds,
//! and refuses anything else.
//!
//! This module is not feature-gated: the route table exists in every build, so
//! the allow-list, the size cap and the filename rules must exist there too.
//! The Python reference (`kala_agent.studio.design.artifacts`) mirrors every
//! rule here byte for byte, and a differential case compares the downloads.

/// The artifact kinds a stored design can answer for a download.
pub const ARTIFACT_KINDS: [&str; 4] = ["script", "step", "fcstd", "stl"];

/// The largest artifact the download route serves.
///
/// The journal's own per-artifact ceiling is `max(64 MiB, the STEP import
/// ceiling)`; this is the hard download cap.
pub const MAX_ARTIFACT_DOWNLOAD_BYTES: usize = 64 * 1024 * 1024;

/// The fallback download filename stem when a design declares no usable label.
pub const DEFAULT_ARTIFACT_SLUG: &str = "kala-studio";

/// The longest label slug the download filename uses.
pub const MAX_ARTIFACT_SLUG_LENGTH: usize = 40;

/// The `content-type` of one downloadable artifact kind.
#[must_use]
pub fn content_type(kind: &str) -> &'static str {
    match kind {
        "script" => "application/json",
        "step" => "application/step",
        "stl" => "model/stl",
        _ => "application/octet-stream",
    }
}

/// The `content-disposition` filename of one downloadable artifact.
///
/// The shape is `<label-slug>-<digest8>.<extension>`; the extension is the
/// provider's own for a native document (`FCStd`).
#[must_use]
pub fn filename(label: &str, digest: &str, kind: &str) -> String {
    let slug = slug(label);
    let short: String = digest.chars().take(8).collect();
    let extension = match kind {
        "script" => "json",
        "step" => "step",
        "stl" => "stl",
        _ => "FCStd",
    };
    format!("{slug}-{short}.{extension}")
}

/// A filesystem- and header-safe slug of a design label.
///
/// ASCII alphanumerics are lowercased; every other run becomes one `-`. The
/// result is bounded to [`MAX_ARTIFACT_SLUG_LENGTH`] characters and falls back
/// to [`DEFAULT_ARTIFACT_SLUG`] when the label has no usable character, so a
/// hostile label can never traverse a path or smuggle a header.
#[must_use]
pub fn slug(label: &str) -> String {
    let mut out = String::new();
    for character in label.chars() {
        if character.is_ascii_alphanumeric() {
            out.push(character.to_ascii_lowercase());
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    let trimmed = out.trim_matches('-');
    let bounded: String = trimmed.chars().take(MAX_ARTIFACT_SLUG_LENGTH).collect();
    let bounded = bounded.trim_matches('-');
    if bounded.is_empty() {
        DEFAULT_ARTIFACT_SLUG.to_owned()
    } else {
        bounded.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_artifact_kind_has_a_stable_content_type_and_extension() {
        assert_eq!(content_type("script"), "application/json");
        assert_eq!(content_type("step"), "application/step");
        assert_eq!(content_type("fcstd"), "application/octet-stream");
        assert_eq!(content_type("stl"), "model/stl");
        assert_eq!(
            filename("L-bracket 60 x 45 x 40", "a9d04f8f0000", "fcstd"),
            "l-bracket-60-x-45-x-40-a9d04f8f.FCStd"
        );
        assert_eq!(
            filename("corpus bracket", "7a50e9c4ffff", "step"),
            "corpus-bracket-7a50e9c4.step"
        );
        assert_eq!(filename("", "abc", "script"), "kala-studio-abc.json");
    }

    #[test]
    fn a_hostile_label_cannot_escape_the_filename() {
        assert_eq!(slug("../../etc/passwd"), "etc-passwd");
        assert_eq!(slug("a\"b\r\nc"), "a-b-c");
        assert_eq!(slug("---"), DEFAULT_ARTIFACT_SLUG);
        assert_eq!(slug("Ünïcode part"), "n-code-part");
        assert!(slug(&"x".repeat(200)).chars().count() <= MAX_ARTIFACT_SLUG_LENGTH);
    }
}
