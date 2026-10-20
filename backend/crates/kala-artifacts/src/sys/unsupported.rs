//! Fails closed on targets without a hardened implementation.

use std::path::Path;

use crate::ArtifactError;

#[derive(Debug)]
pub(crate) struct Dir;

pub(crate) struct Bucket<'a> {
    _objects: &'a Dir,
    _name: String,
}

impl Dir {
    pub(crate) fn open_root(_path: &Path) -> Result<Self, ArtifactError> {
        Err(ArtifactError::UnsupportedPlatform)
    }

    pub(crate) fn ensure_objects(&self, _root: &Path) -> Result<Self, ArtifactError> {
        Err(ArtifactError::UnsupportedPlatform)
    }

    pub(crate) fn verify_objects_path(
        &self,
        _expected: &Dir,
        _root: &Path,
    ) -> Result<(), ArtifactError> {
        Err(ArtifactError::UnsupportedPlatform)
    }

    pub(crate) fn open_bucket<'a>(
        &'a self,
        _bucket: &str,
        _digest: &str,
        _create: bool,
        _max_object_size: u64,
    ) -> Result<Bucket<'a>, ArtifactError> {
        Err(ArtifactError::UnsupportedPlatform)
    }
}

impl Bucket<'_> {
    pub(crate) fn read_object(&self, _digest: &str) -> Result<Vec<u8>, ArtifactError> {
        Err(ArtifactError::UnsupportedPlatform)
    }

    pub(crate) fn publish(&self, _digest: &str, _data: &[u8]) -> Result<(), ArtifactError> {
        Err(ArtifactError::UnsupportedPlatform)
    }
}
