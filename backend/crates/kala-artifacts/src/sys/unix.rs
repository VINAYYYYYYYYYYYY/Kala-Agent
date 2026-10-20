//! Descriptor-relative Unix implementation.
//!
//! All component opens are relative to an already-open directory descriptor.
//! `O_NOFOLLOW` is set on every final component, `O_DIRECTORY` on directories,
//! `O_NONBLOCK` on object reads so a FIFO cannot hang the caller, and `O_EXCL`
//! on temporary creation.  Publication is `linkat` within the same bucket
//! descriptor, so it is atomic and cannot overwrite an existing digest.

use std::fs::{self, File};
use std::io::{self, Write};
use std::os::fd::OwnedFd;
use std::path::Path;

use rustix::fs::{AtFlags, FileType, Mode, OFlags};
use rustix::io::Errno;

use crate::{
    ArtifactError, random_temporary_name, read_limited, temporary_name_attempts, verify_digest,
};

const DIRECTORY_OPEN_FLAGS: OFlags = OFlags::RDONLY
    .union(OFlags::DIRECTORY)
    .union(OFlags::NOFOLLOW)
    .union(OFlags::CLOEXEC);
const OBJECT_READ_FLAGS: OFlags = OFlags::RDONLY
    .union(OFlags::NONBLOCK)
    .union(OFlags::NOFOLLOW)
    .union(OFlags::CLOEXEC);
const TEMPORARY_WRITE_FLAGS: OFlags = OFlags::WRONLY
    .union(OFlags::CREATE)
    .union(OFlags::EXCL)
    .union(OFlags::NOFOLLOW)
    .union(OFlags::CLOEXEC);

pub(crate) struct Dir(OwnedFd);

impl std::fmt::Debug for Dir {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_tuple("Dir").field(&self.0).finish()
    }
}

pub(crate) struct Bucket<'a> {
    name: String,
    objects: &'a Dir,
    dir: Dir,
    max_object_size: u64,
}

impl Dir {
    pub(crate) fn open_root(path: &Path) -> Result<Self, ArtifactError> {
        match rustix::fs::openat(
            rustix::fs::CWD,
            path.as_os_str(),
            DIRECTORY_OPEN_FLAGS,
            Mode::empty(),
        ) {
            Ok(fd) => Ok(Self(fd)),
            Err(Errno::LOOP) => {
                if fs::symlink_metadata(path)
                    .map(|metadata| metadata.file_type().is_symlink())
                    .unwrap_or(false)
                {
                    Err(ArtifactError::RootSymlink)
                } else {
                    Err(ArtifactError::Io(io::Error::from(Errno::LOOP)))
                }
            }
            Err(Errno::NOTDIR) => Err(ArtifactError::RootNotDirectory(path.to_path_buf())),
            Err(error) => Err(ArtifactError::Io(error.into())),
        }
    }

    pub(crate) fn ensure_objects(&self, root: &Path) -> Result<Self, ArtifactError> {
        match rustix::fs::mkdirat(&self.0, "objects", Mode::RWXU) {
            Ok(()) => {}
            Err(Errno::EXIST) => {}
            Err(error) => return Err(ArtifactError::Io(error.into())),
        }

        match rustix::fs::statat(&self.0, "objects", AtFlags::SYMLINK_NOFOLLOW) {
            Ok(stat) => {
                let file_type = FileType::from_raw_mode(stat.st_mode as _);
                if file_type.is_symlink() {
                    return Err(ArtifactError::ObjectRootSymlink);
                }
                if !file_type.is_dir() {
                    return Err(ArtifactError::ObjectRootNotDirectory(root.join("objects")));
                }
            }
            Err(error) => return Err(ArtifactError::Io(error.into())),
        }

        match rustix::fs::openat(&self.0, "objects", DIRECTORY_OPEN_FLAGS, Mode::empty()) {
            Ok(fd) => Ok(Self(fd)),
            Err(Errno::LOOP) => Err(ArtifactError::ObjectRootSymlink),
            Err(Errno::NOTDIR) => Err(classify_unusable_entry(
                self,
                "objects",
                ArtifactError::ObjectRootSymlink,
                ArtifactError::ObjectRootNotDirectory(root.join("objects")),
            )),
            Err(error) => Err(ArtifactError::Io(error.into())),
        }
    }

    pub(crate) fn verify_objects_path(
        &self,
        expected: &Dir,
        root: &Path,
    ) -> Result<(), ArtifactError> {
        let current =
            match rustix::fs::openat(&self.0, "objects", DIRECTORY_OPEN_FLAGS, Mode::empty()) {
                Ok(fd) => Self(fd),
                Err(Errno::LOOP) => return Err(ArtifactError::ObjectRootSymlink),
                Err(Errno::NOTDIR) => {
                    return Err(classify_unusable_entry(
                        self,
                        "objects",
                        ArtifactError::ObjectRootSymlink,
                        ArtifactError::ObjectRootNotDirectory(root.join("objects")),
                    ));
                }
                Err(error) => return Err(ArtifactError::Io(error.into())),
            };
        let current_stat = rustix::fs::fstat(&current.0)?;
        let expected_stat = rustix::fs::fstat(&expected.0)?;
        if current_stat.st_dev != expected_stat.st_dev
            || current_stat.st_ino != expected_stat.st_ino
        {
            return Err(ArtifactError::ObjectRootReplaced);
        }
        Ok(())
    }

    pub(crate) fn open_bucket<'a>(
        &'a self,
        bucket: &str,
        digest: &str,
        create: bool,
        max_object_size: u64,
    ) -> Result<Bucket<'a>, ArtifactError> {
        if create {
            match rustix::fs::mkdirat(&self.0, bucket, Mode::RWXU) {
                Ok(()) => {}
                Err(Errno::EXIST) => {
                    match rustix::fs::statat(&self.0, bucket, AtFlags::SYMLINK_NOFOLLOW) {
                        Ok(stat) => {
                            let file_type = FileType::from_raw_mode(stat.st_mode as _);
                            if file_type.is_symlink() {
                                return Err(ArtifactError::ObjectPathSymlink);
                            }
                            if !file_type.is_dir() {
                                return Err(ArtifactError::ObjectParentNotDirectory);
                            }
                        }
                        Err(_) => return Err(ArtifactError::ObjectParentNotDirectory),
                    }
                }
                Err(error) => return Err(ArtifactError::Io(error.into())),
            }
        }

        let fd = match rustix::fs::openat(&self.0, bucket, DIRECTORY_OPEN_FLAGS, Mode::empty()) {
            Ok(fd) => fd,
            Err(Errno::NOENT) => return Err(ArtifactError::NotFound(digest.to_string())),
            Err(Errno::LOOP) => return Err(ArtifactError::ObjectPathSymlink),
            Err(Errno::NOTDIR) => {
                return Err(classify_unusable_entry(
                    self,
                    bucket,
                    ArtifactError::ObjectPathSymlink,
                    ArtifactError::ObjectParentNotDirectory,
                ));
            }
            Err(error) => return Err(ArtifactError::Io(error.into())),
        };
        Ok(Bucket {
            name: bucket.to_string(),
            objects: self,
            dir: Self(fd),
            max_object_size,
        })
    }
}

impl Bucket<'_> {
    pub(crate) fn read_object(&self, digest: &str) -> Result<Vec<u8>, ArtifactError> {
        let fd = match rustix::fs::openat(&self.dir.0, digest, OBJECT_READ_FLAGS, Mode::empty()) {
            Ok(fd) => fd,
            Err(Errno::NOENT) => return Err(ArtifactError::NotFound(digest.to_string())),
            Err(Errno::LOOP) => return Err(ArtifactError::ObjectSymlink),
            Err(_) => return Err(ArtifactError::ObjectOpen),
        };
        let file = File::from(fd);
        let stat = rustix::fs::fstat(&file)?;
        if !FileType::from_raw_mode(stat.st_mode as _).is_file() {
            return Err(ArtifactError::NonRegular);
        }
        let data = read_limited(file, self.max_object_size)?;
        verify_digest(digest, &data)?;
        Ok(data)
    }

    pub(crate) fn publish(&self, digest: &str, data: &[u8]) -> Result<(), ArtifactError> {
        let temporary = self.write_temporary_object(data)?;
        let outcome = self.link_temporary_object(&temporary, digest, data);
        // The temporary name is always removed, including after a failed
        // publication.  Like Python's `finally`, a cleanup failure is reported.
        let cleanup = self.remove_entry(&temporary);
        cleanup?;
        outcome?;
        self.verify_identity()
    }

    fn write_temporary_object(&self, data: &[u8]) -> Result<String, ArtifactError> {
        for _ in 0..temporary_name_attempts() {
            let name = random_temporary_name()?;
            let fd = match rustix::fs::openat(
                &self.dir.0,
                name.as_str(),
                TEMPORARY_WRITE_FLAGS,
                Mode::RUSR.union(Mode::WUSR),
            ) {
                Ok(fd) => fd,
                Err(Errno::EXIST) => continue,
                Err(error) => return Err(ArtifactError::Io(error.into())),
            };
            let mut file = File::from(fd);
            let write_result = file.write_all(data).and_then(|()| file.sync_all());
            drop(file);
            if let Err(error) = write_result {
                let cleanup = self.remove_entry(&name);
                cleanup?;
                return Err(ArtifactError::Io(error));
            }
            return Ok(name);
        }
        Err(ArtifactError::TemporaryNameExhausted)
    }

    fn link_temporary_object(
        &self,
        temporary: &str,
        digest: &str,
        data: &[u8],
    ) -> Result<(), ArtifactError> {
        match rustix::fs::linkat(
            &self.dir.0,
            temporary,
            &self.dir.0,
            digest,
            AtFlags::empty(),
        ) {
            Ok(()) => {
                sync_directory(&self.dir)?;
                if !entry_is_regular_file(&self.dir, digest) {
                    return Err(ArtifactError::PublicationFailed);
                }
                Ok(())
            }
            Err(Errno::EXIST) => {
                let existing = match self.read_object(digest) {
                    Ok(existing) => existing,
                    Err(ArtifactError::NotFound(_)) => {
                        return Err(ArtifactError::PublicationFailed);
                    }
                    Err(error) => return Err(error),
                };
                if existing != data {
                    return Err(ArtifactError::ExistingIntegrity);
                }
                Ok(())
            }
            Err(Errno::NOENT) => Err(ArtifactError::PublicationFailed),
            Err(error) => Err(ArtifactError::Io(error.into())),
        }
    }

    fn remove_entry(&self, name: &str) -> Result<(), ArtifactError> {
        match rustix::fs::unlinkat(&self.dir.0, name, AtFlags::empty()) {
            Ok(()) => Ok(()),
            Err(Errno::NOENT) => Ok(()),
            Err(error) => Err(ArtifactError::Io(error.into())),
        }
    }

    fn verify_identity(&self) -> Result<(), ArtifactError> {
        let reached = rustix::fs::statat(
            &self.objects.0,
            self.name.as_str(),
            AtFlags::SYMLINK_NOFOLLOW,
        )
        .map_err(|_| ArtifactError::PublicationFailed)?;
        let opened =
            rustix::fs::fstat(&self.dir.0).map_err(|_| ArtifactError::PublicationFailed)?;
        if reached.st_dev != opened.st_dev || reached.st_ino != opened.st_ino {
            return Err(ArtifactError::PublicationFailed);
        }
        Ok(())
    }
}

fn entry_is_regular_file(directory: &Dir, name: &str) -> bool {
    match rustix::fs::statat(&directory.0, name, AtFlags::SYMLINK_NOFOLLOW) {
        Ok(stat) => FileType::from_raw_mode(stat.st_mode as _).is_file(),
        Err(_) => false,
    }
}

/// Classify an entry whose directory open failed with `ENOTDIR`/`ELOOP`.
///
/// Linux can report either errno for a symlink depending on the open flags, so
/// the entry's own `lstat` decides whether the failure is a symlink refusal or
/// a not-a-directory refusal.
fn classify_unusable_entry(
    directory: &Dir,
    name: &str,
    symlink_error: ArtifactError,
    not_directory_error: ArtifactError,
) -> ArtifactError {
    match rustix::fs::statat(&directory.0, name, AtFlags::SYMLINK_NOFOLLOW) {
        Ok(stat) if FileType::from_raw_mode(stat.st_mode as _).is_symlink() => symlink_error,
        _ => not_directory_error,
    }
}

fn sync_directory(directory: &Dir) -> Result<(), ArtifactError> {
    match rustix::fs::fsync(&directory.0) {
        Ok(()) => Ok(()),
        Err(Errno::INVAL | Errno::NOSYS | Errno::PERM | Errno::ACCESS | Errno::ROFS) => Ok(()),
        Err(error) => Err(ArtifactError::Io(error.into())),
    }
}
