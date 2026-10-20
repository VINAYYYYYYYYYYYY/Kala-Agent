//! Native Windows implementation using handle-relative NT file APIs.
//!
//! The root and `objects` directories are opened once and held as handles.
//! Every child is created or opened with `NtCreateFile` and an
//! `OBJECT_ATTRIBUTES.RootDirectory` pointing at the already-validated parent,
//! so no component is re-resolved by path and a renamed or symlinked path can
//! not redirect an operation.  `FILE_OPEN_REPARSE_POINT` is always set and the
//! resulting `FileAttributeTagInfo` is checked, so a symlink or junction is
//! never traversed.  Publication renames the flushed temporary file by handle
//! with `ReplaceIfExists = false`, which is atomic and refuses to overwrite an
//! existing digest; the bucket identity is re-read by file ID afterwards.
//!
//! The rename target is bound to the opened bucket by setting
//! `FILE_RENAME_INFO.RootDirectory` to the bucket handle and passing only the
//! bare digest name.  Microsoft documents that a relative `FileName` with a
//! non-NULL `RootDirectory` is resolved against that directory handle, whereas
//! a relative name with `RootDirectory == NULL` is resolved against the
//! process's current directory (see the evidence recorded in `REPORT.md`).
//! Therefore the target does not depend on the process cwd or on later renames
//! of the root/bucket paths.
//!
//! Known Windows guarantee differences, stated rather than hidden:
//! * NTFS does not expose a portable directory `fsync`.  Data is flushed with
//!   `FlushFileBuffers` on the temporary file; the directory flush is attempted
//!   and `ACCESS_DENIED`/`INVALID_HANDLE`/`NOT_SUPPORTED` are tolerated, so
//!   entry-name durability depends on the filesystem's own metadata ordering.
//! * Directory/file DACLs are inherited from the parent directory.  This crate
//!   does not loosen or forge an ACL and does not claim the Python `0o700/0o600`
//!   mode; on Windows the effective guarantee is whatever ACL the application
//!   data directory already carries.

use std::ffi::c_void;
use std::fs::File;
use std::io::{self, Write};
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::path::Path;
use std::ptr::{addr_of_mut, null, null_mut};

use windows_sys::Wdk::Foundation::OBJECT_ATTRIBUTES;
use windows_sys::Wdk::Storage::FileSystem::{
    FILE_CREATE, FILE_DIRECTORY_FILE, FILE_NON_DIRECTORY_FILE, FILE_OPEN, FILE_OPEN_IF,
    FILE_OPEN_REPARSE_POINT, FILE_SYNCHRONOUS_IO_NONALERT, NtCreateFile,
    RtlNtStatusToDosErrorNoTeb,
};
use windows_sys::Win32::Foundation::{
    ERROR_ALREADY_EXISTS, ERROR_FILE_EXISTS, ERROR_FILE_NOT_FOUND, ERROR_PATH_NOT_FOUND,
    GENERIC_READ, GENERIC_WRITE, GetLastError, HANDLE, INVALID_HANDLE_VALUE, UNICODE_STRING,
};
use windows_sys::Win32::Storage::FileSystem::{
    BY_HANDLE_FILE_INFORMATION, CreateFileW, DELETE, FILE_ADD_FILE, FILE_ADD_SUBDIRECTORY,
    FILE_ATTRIBUTE_DEVICE, FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_NORMAL,
    FILE_ATTRIBUTE_REPARSE_POINT, FILE_ATTRIBUTE_TAG_INFO, FILE_DISPOSITION_INFO,
    FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_ID_INFO, FILE_LIST_DIRECTORY,
    FILE_READ_ATTRIBUTES, FILE_RENAME_INFO, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
    FILE_TRAVERSE, FileAttributeTagInfo, FileDispositionInfo, FileIdInfo, FileRenameInfo,
    FlushFileBuffers, GetFileInformationByHandle, GetFileInformationByHandleEx, OPEN_EXISTING,
    SYNCHRONIZE, SetFileInformationByHandle,
};
use windows_sys::Win32::System::IO::IO_STATUS_BLOCK;

use crate::{
    ArtifactError, random_temporary_name, read_limited, temporary_name_attempts, verify_digest,
};

const DIR_READ_ACCESS: u32 =
    FILE_LIST_DIRECTORY | FILE_TRAVERSE | FILE_READ_ATTRIBUTES | SYNCHRONIZE;
const DIR_WRITE_ACCESS: u32 = DIR_READ_ACCESS | FILE_ADD_FILE | FILE_ADD_SUBDIRECTORY;
const TEMPORARY_ACCESS: u32 = GENERIC_WRITE | DELETE | FILE_READ_ATTRIBUTES | SYNCHRONIZE;
const OBJECT_ACCESS: u32 = GENERIC_READ | FILE_READ_ATTRIBUTES | SYNCHRONIZE;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Identity {
    volume: u64,
    id: [u8; 16],
}

pub(crate) struct Dir(OwnedHandle);

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
    fn raw(&self) -> HANDLE {
        self.0.as_raw_handle()
    }

    fn attributes(&self) -> io::Result<u32> {
        file_attributes(self.raw())
    }

    fn identity(&self) -> io::Result<Identity> {
        file_identity(self.raw())
    }

    fn is_reparse(&self) -> io::Result<bool> {
        Ok(self.attributes()? & FILE_ATTRIBUTE_REPARSE_POINT != 0)
    }

    fn is_dir(&self) -> io::Result<bool> {
        Ok(self.attributes()? & FILE_ATTRIBUTE_DIRECTORY != 0)
    }

    fn is_regular_file(&self) -> io::Result<bool> {
        let attributes = self.attributes()?;
        Ok(attributes
            & (FILE_ATTRIBUTE_DIRECTORY | FILE_ATTRIBUTE_REPARSE_POINT | FILE_ATTRIBUTE_DEVICE)
            == 0)
    }

    pub(crate) fn open_root(path: &Path) -> Result<Self, ArtifactError> {
        let dir = match open_root_handle(path, DIR_WRITE_ACCESS) {
            Ok(dir) => dir,
            Err(error)
                if error.raw_os_error()
                    == Some(windows_sys::Win32::Foundation::ERROR_ACCESS_DENIED as i32) =>
            {
                // A read-only store can still serve verified reads; bucket and
                // object creation will fail closed with the original error.
                match open_root_handle(path, DIR_READ_ACCESS) {
                    Ok(dir) => dir,
                    Err(error) => return Err(ArtifactError::Io(error)),
                }
            }
            Err(error) => return Err(ArtifactError::Io(error)),
        };
        if dir.is_reparse()? {
            return Err(ArtifactError::RootSymlink);
        }
        if !dir.is_dir()? {
            return Err(ArtifactError::RootNotDirectory(path.to_path_buf()));
        }
        Ok(dir)
    }

    pub(crate) fn ensure_objects(&self, root: &Path) -> Result<Self, ArtifactError> {
        let dir = match open_child(
            self,
            "objects",
            FILE_OPEN_IF,
            FILE_DIRECTORY_FILE | FILE_OPEN_REPARSE_POINT | FILE_SYNCHRONOUS_IO_NONALERT,
            DIR_WRITE_ACCESS,
        ) {
            Ok(dir) => dir,
            Err(create_error) => {
                match open_child(self, "objects", FILE_OPEN, open_flags(), DIR_READ_ACCESS) {
                    Ok(dir) => dir,
                    Err(_) => return Err(ArtifactError::Io(create_error)),
                }
                // The directory already exists but this handle was opened
                // without create rights (for example a read-only ACL); keep
                // the readable handle.  The unconditional check below validates
                // whichever handle this branch selected.
            }
        };
        if dir.is_reparse()? {
            return Err(ArtifactError::ObjectRootSymlink);
        }
        if !dir.is_dir()? {
            return Err(ArtifactError::ObjectRootNotDirectory(root.join("objects")));
        }
        Ok(dir)
    }

    pub(crate) fn verify_objects_path(
        &self,
        expected: &Dir,
        root: &Path,
    ) -> Result<(), ArtifactError> {
        let current = open_child(self, "objects", FILE_OPEN, open_flags(), DIR_READ_ACCESS)?;
        if current.is_reparse()? {
            return Err(ArtifactError::ObjectRootSymlink);
        }
        if !current.is_dir()? {
            return Err(ArtifactError::ObjectRootNotDirectory(root.join("objects")));
        }
        if current.identity()? != expected.identity()? {
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
        let dir = if create {
            match open_child(
                self,
                bucket,
                FILE_OPEN_IF,
                FILE_DIRECTORY_FILE | FILE_OPEN_REPARSE_POINT | FILE_SYNCHRONOUS_IO_NONALERT,
                DIR_WRITE_ACCESS,
            ) {
                Ok(dir) => dir,
                Err(create_error) => {
                    // A concurrent writer created the bucket while this one
                    // lost the creation race, or the ACL denies create rights.
                    // Reopen the existing directory for write access.
                    reopen_bucket_for_write(self, bucket, create_error)?
                }
            }
        } else {
            match open_child(self, bucket, FILE_OPEN, open_flags(), DIR_READ_ACCESS) {
                Ok(dir) => dir,
                Err(error) if is_not_found(&error) => {
                    return Err(ArtifactError::NotFound(digest.to_string()));
                }
                Err(error) => return Err(ArtifactError::Io(error)),
            }
        };
        // Every branch selected above must pass the same final reparse-point
        // and directory checks before the handle can become a publication or
        // read parent.  In particular this covers the successful
        // `FILE_OPEN_IF` path and the write-access fallback reopen.
        validate_bucket_directory(&dir)?;
        Ok(Bucket {
            name: bucket.to_string(),
            objects: self,
            dir,
            max_object_size,
        })
    }
}

/// Reject a selected bucket handle that is a reparse point or not a directory.
///
/// Kept as one function so the check is identical for the read branch, the
/// `FILE_OPEN_IF` success branch and the write-access fallback reopen.
fn validate_bucket_directory(dir: &Dir) -> Result<(), ArtifactError> {
    if dir.is_reparse()? {
        return Err(ArtifactError::ObjectPathSymlink);
    }
    if !dir.is_dir()? {
        return Err(ArtifactError::ObjectParentNotDirectory);
    }
    Ok(())
}

fn reopen_bucket_for_write(
    parent: &Dir,
    bucket: &str,
    create_error: io::Error,
) -> Result<Dir, ArtifactError> {
    let existing = match open_child(parent, bucket, FILE_OPEN, open_flags(), DIR_READ_ACCESS) {
        Ok(dir) => dir,
        Err(_) => return Err(ArtifactError::Io(create_error)),
    };
    validate_bucket_directory(&existing)?;
    drop(existing);
    // Test-only seam: a unit test replaces the bucket between the readable
    // open and the writable reopen to prove the final handle is validated.
    run_bucket_write_reopen_hook();
    let dir = open_child(parent, bucket, FILE_OPEN, open_flags(), DIR_WRITE_ACCESS)
        .map_err(ArtifactError::Io)?;
    // Validate the writable handle itself, not the earlier readable handle:
    // the path may have been replaced at any point between the two opens.
    validate_bucket_directory(&dir)?;
    Ok(dir)
}

#[cfg(test)]
thread_local! {
    static BUCKET_WRITE_REOPEN_HOOK: std::cell::RefCell<Option<Box<dyn FnOnce()>>> =
        const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
fn run_bucket_write_reopen_hook() {
    BUCKET_WRITE_REOPEN_HOOK.with(|hook| {
        if let Some(hook) = hook.borrow_mut().take() {
            hook();
        }
    });
}

#[cfg(not(test))]
fn run_bucket_write_reopen_hook() {}

impl Bucket<'_> {
    pub(crate) fn read_object(&self, digest: &str) -> Result<Vec<u8>, ArtifactError> {
        let handle = match open_child(&self.dir, digest, FILE_OPEN, open_flags(), OBJECT_ACCESS) {
            Ok(dir) => dir,
            Err(error) if is_not_found(&error) => {
                return Err(ArtifactError::NotFound(digest.to_string()));
            }
            Err(_) => return Err(ArtifactError::ObjectOpen),
        };
        if handle.is_reparse()? {
            return Err(ArtifactError::ObjectSymlink);
        }
        if !handle.is_regular_file()? {
            return Err(ArtifactError::NonRegular);
        }
        let file = File::from(handle.0);
        let data = read_limited(file, self.max_object_size)?;
        verify_digest(digest, &data)?;
        Ok(data)
    }

    pub(crate) fn publish(&self, digest: &str, data: &[u8]) -> Result<(), ArtifactError> {
        let (_temporary, file) = self.write_temporary_object(data)?;
        let mut renamed = false;
        let outcome = (|| -> Result<(), ArtifactError> {
            match rename_by_handle(&self.dir, &file, digest) {
                Ok(()) => {
                    renamed = true;
                    sync_directory(&self.dir)?;
                    if !entry_is_regular_file(&self.dir, digest) {
                        return Err(ArtifactError::PublicationFailed);
                    }
                    Ok(())
                }
                Err(error) if is_collision(&error) => {
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
                Err(error) if is_not_found(&error) => Err(ArtifactError::PublicationFailed),
                Err(error) => Err(ArtifactError::Io(error)),
            }
        })();

        let cleanup = if renamed {
            Ok(())
        } else {
            dispose_by_handle(&file)
        };
        drop(file);
        if let Err(error) = cleanup {
            return Err(ArtifactError::Io(error));
        }
        outcome?;
        self.verify_identity()
    }

    fn write_temporary_object(&self, data: &[u8]) -> Result<(String, File), ArtifactError> {
        for _ in 0..temporary_name_attempts() {
            let name = random_temporary_name()?;
            let handle = match open_child(
                &self.dir,
                name.as_str(),
                FILE_CREATE,
                FILE_NON_DIRECTORY_FILE | FILE_OPEN_REPARSE_POINT | FILE_SYNCHRONOUS_IO_NONALERT,
                TEMPORARY_ACCESS,
            ) {
                Ok(handle) => handle,
                Err(error) if is_collision(&error) => continue,
                Err(error) => return Err(ArtifactError::Io(error)),
            };
            let file = File::from(handle.0);
            let mut file = file;
            if let Err(error) = file.write_all(data).and_then(|()| file.sync_all()) {
                let cleanup = dispose_by_handle(&file);
                drop(file);
                if let Err(cleanup_error) = cleanup {
                    return Err(ArtifactError::Io(cleanup_error));
                }
                return Err(ArtifactError::Io(error));
            }
            return Ok((name, file));
        }
        Err(ArtifactError::TemporaryNameExhausted)
    }

    fn verify_identity(&self) -> Result<(), ArtifactError> {
        let current = match open_child(
            self.objects,
            self.name.as_str(),
            FILE_OPEN,
            open_flags(),
            DIR_READ_ACCESS,
        ) {
            Ok(dir) => dir,
            Err(_) => return Err(ArtifactError::PublicationFailed),
        };
        if current.is_reparse()? || !current.is_dir()? {
            return Err(ArtifactError::PublicationFailed);
        }
        match (current.identity(), self.dir.identity()) {
            (Ok(current), Ok(opened)) if current == opened => Ok(()),
            _ => Err(ArtifactError::PublicationFailed),
        }
    }
}

fn open_flags() -> u32 {
    FILE_OPEN_REPARSE_POINT | FILE_SYNCHRONOUS_IO_NONALERT
}

fn open_root_handle(path: &Path, access: u32) -> io::Result<Dir> {
    let wide = to_wide_null(path);
    // SAFETY: `wide` is a NUL-terminated UTF-16 path, the security attributes
    // and template handle are null as documented, and the returned handle is
    // checked against `INVALID_HANDLE_VALUE`.
    let handle = unsafe {
        CreateFileW(
            wide.as_ptr(),
            access,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            null(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
            null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(last_error());
    }
    // SAFETY: `handle` is a valid, uniquely owned handle returned above.
    Ok(Dir(unsafe { OwnedHandle::from_raw_handle(handle) }))
}

fn open_child(
    parent: &Dir,
    name: &str,
    disposition: u32,
    options: u32,
    access: u32,
) -> io::Result<Dir> {
    let wide: Vec<u16> = name.encode_utf16().collect();
    let unicode = UNICODE_STRING {
        Length: (wide.len() * 2) as u16,
        MaximumLength: ((wide.len() + 1) * 2) as u16,
        Buffer: wide.as_ptr() as *mut u16,
    };
    let attributes = OBJECT_ATTRIBUTES {
        Length: std::mem::size_of::<OBJECT_ATTRIBUTES>() as u32,
        RootDirectory: parent.raw(),
        ObjectName: &unicode,
        Attributes: 0,
        SecurityDescriptor: null(),
        SecurityQualityOfService: null(),
    };
    let mut handle: HANDLE = null_mut();
    let mut status_block = IO_STATUS_BLOCK::default();
    // SAFETY: `attributes` and `status_block` are initialized writable
    // structures, `unicode` points at `wide` for the duration of the call, and
    // `handle` receives the result when the returned NTSTATUS is non-negative.
    let status = unsafe {
        NtCreateFile(
            &mut handle,
            access,
            &attributes,
            &mut status_block,
            null(),
            FILE_ATTRIBUTE_NORMAL,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            disposition,
            options,
            null(),
            0,
        )
    };
    if status < 0 {
        // SAFETY: `RtlNtStatusToDosErrorNoTeb` accepts any NTSTATUS value.
        let code = unsafe { RtlNtStatusToDosErrorNoTeb(status) };
        return Err(io::Error::from_raw_os_error(code as i32));
    }
    // SAFETY: a non-negative status means `handle` is a valid, uniquely owned
    // handle.
    Ok(Dir(unsafe { OwnedHandle::from_raw_handle(handle) }))
}

fn file_attributes(handle: HANDLE) -> io::Result<u32> {
    let mut info = FILE_ATTRIBUTE_TAG_INFO::default();
    // SAFETY: `info` is a writable `FILE_ATTRIBUTE_TAG_INFO` and `handle` is a
    // valid handle owned by the caller.
    let result = unsafe {
        GetFileInformationByHandleEx(
            handle,
            FileAttributeTagInfo,
            (&mut info as *mut FILE_ATTRIBUTE_TAG_INFO).cast(),
            std::mem::size_of::<FILE_ATTRIBUTE_TAG_INFO>() as u32,
        )
    };
    if result == 0 {
        return Err(last_error());
    }
    Ok(info.FileAttributes)
}

fn file_identity(handle: HANDLE) -> io::Result<Identity> {
    let mut info = FILE_ID_INFO::default();
    // SAFETY: `info` and `handle` are valid as in `file_attributes`.
    let result = unsafe {
        GetFileInformationByHandleEx(
            handle,
            FileIdInfo,
            (&mut info as *mut FILE_ID_INFO).cast(),
            std::mem::size_of::<FILE_ID_INFO>() as u32,
        )
    };
    if result != 0 {
        return Ok(Identity {
            volume: info.VolumeSerialNumber,
            id: info.FileId.Identifier,
        });
    }
    let mut basic = BY_HANDLE_FILE_INFORMATION::default();
    // SAFETY: `basic` and `handle` are valid as in `file_attributes`.
    let fallback = unsafe { GetFileInformationByHandle(handle, &mut basic) };
    if fallback == 0 {
        return Err(last_error());
    }
    let mut id = [0u8; 16];
    id[..4].copy_from_slice(&basic.nFileIndexHigh.to_le_bytes());
    id[4..8].copy_from_slice(&basic.nFileIndexLow.to_le_bytes());
    Ok(Identity {
        volume: u64::from(basic.dwVolumeSerialNumber),
        id,
    })
}

fn rename_by_handle(directory: &Dir, file: &File, digest: &str) -> io::Result<()> {
    let name: Vec<u16> = digest.encode_utf16().collect();
    // Allocate like the documented `sizeof(FILE_RENAME_INFO) + FileNameLength`
    // shape.  `size_of` already covers the flexible one-character `FileName`
    // field and its trailing padding, so this is never smaller than the header
    // plus the NUL-terminated name.
    let total_size = std::mem::size_of::<FILE_RENAME_INFO>() + name.len() * 2;
    debug_assert!(
        total_size >= std::mem::offset_of!(FILE_RENAME_INFO, FileName) + (name.len() + 1) * 2
    );
    let word_size = std::mem::size_of::<FILE_RENAME_INFO>();
    let mut buffer = vec![FILE_RENAME_INFO::default(); total_size.div_ceil(word_size)];
    let header = buffer.as_mut_ptr();
    // SAFETY: `buffer` is allocated and aligned for `FILE_RENAME_INFO`, and the
    // destination starts at the flexible `FileName` field.  The buffer includes
    // the terminating NUL; `FileNameLength` excludes it, as documented.
    unsafe {
        // `FileRenameInfo` (not `FileRenameInfoEx`) reads `ReplaceIfExists`.
        (*header).Anonymous.ReplaceIfExists = false;
        // Relative names are resolved against this directory handle, so the
        // target is the opened bucket regardless of the process cwd or later
        // renames of the root/bucket path.
        (*header).RootDirectory = directory.raw();
        (*header).FileNameLength = (name.len() * 2) as u32;
        let destination = addr_of_mut!((*header).FileName).cast::<u16>();
        std::ptr::copy_nonoverlapping(name.as_ptr(), destination, name.len());
        *destination.add(name.len()) = 0;
    }
    // SAFETY: `file` owns a valid handle opened with `DELETE`, `directory`
    // owns a valid directory handle, and `buffer` describes a
    // `FILE_RENAME_INFO` whose length covers the header and file name.
    let result = unsafe {
        SetFileInformationByHandle(
            file.as_raw_handle(),
            FileRenameInfo,
            buffer.as_ptr().cast(),
            total_size as u32,
        )
    };
    if result == 0 {
        return Err(last_error());
    }
    Ok(())
}

fn dispose_by_handle(file: &File) -> io::Result<()> {
    let disposition = FILE_DISPOSITION_INFO { DeleteFile: true };
    // SAFETY: `file` owns a valid handle opened with `DELETE`; `disposition`
    // is the documented `FILE_DISPOSITION_INFO` payload.
    let result = unsafe {
        SetFileInformationByHandle(
            file.as_raw_handle(),
            FileDispositionInfo,
            (&disposition as *const FILE_DISPOSITION_INFO).cast::<c_void>(),
            std::mem::size_of::<FILE_DISPOSITION_INFO>() as u32,
        )
    };
    if result == 0 {
        return Err(last_error());
    }
    Ok(())
}

fn entry_is_regular_file(directory: &Dir, name: &str) -> bool {
    match open_child(
        directory,
        name,
        FILE_OPEN,
        open_flags(),
        FILE_READ_ATTRIBUTES | SYNCHRONIZE,
    ) {
        Ok(dir) => dir.is_regular_file().unwrap_or(false),
        Err(_) => false,
    }
}

fn sync_directory(directory: &Dir) -> Result<(), ArtifactError> {
    // SAFETY: `directory.raw()` is a valid directory handle.
    let result = unsafe { FlushFileBuffers(directory.raw()) };
    if result != 0 {
        return Ok(());
    }
    let error = last_error();
    match error.raw_os_error().unwrap_or_default() {
        // ERROR_INVALID_FUNCTION, ERROR_ACCESS_DENIED, ERROR_INVALID_HANDLE,
        // ERROR_NOT_SUPPORTED, ERROR_INVALID_PARAMETER: Win32 has no portable
        // directory fsync and these are the documented "cannot flush a
        // directory handle" results.  Data was already flushed on the file.
        1 | 5 | 6 | 50 | 87 => Ok(()),
        _ => Err(ArtifactError::Io(error)),
    }
}

fn to_wide_null(path: &Path) -> Vec<u16> {
    path.as_os_str().encode_wide().chain(Some(0)).collect()
}

fn last_error() -> io::Error {
    // SAFETY: `GetLastError` is always callable on the calling thread.
    io::Error::from_raw_os_error(unsafe { GetLastError() } as i32)
}

fn is_collision(error: &io::Error) -> bool {
    matches!(
        error.raw_os_error(),
        Some(code) if code == ERROR_ALREADY_EXISTS as i32 || code == ERROR_FILE_EXISTS as i32
    )
}

fn is_not_found(error: &io::Error) -> bool {
    matches!(
        error.raw_os_error(),
        Some(code) if code == ERROR_FILE_NOT_FOUND as i32 || code == ERROR_PATH_NOT_FOUND as i32
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture {
        temp: tempfile::TempDir,
        root_path: std::path::PathBuf,
        objects: Dir,
    }

    fn fixture() -> Fixture {
        let temp = tempfile::tempdir().unwrap();
        let root_path = temp.path().join("store");
        std::fs::create_dir(&root_path).unwrap();
        let root = Dir::open_root(&root_path).unwrap();
        let objects = root.ensure_objects(&root_path).unwrap();
        Fixture {
            temp,
            root_path,
            objects,
        }
    }

    fn expect_error<T>(result: Result<T, ArtifactError>, expected: ArtifactError) {
        match result {
            Ok(_) => panic!("operation unexpectedly succeeded"),
            Err(error) => assert_eq!(error.to_string(), expected.to_string()),
        }
    }

    #[test]
    fn open_bucket_validates_the_file_open_if_success_handle() {
        let fixture = fixture();
        let digest = "a".repeat(64);
        let bucket = &digest[..2];
        let path = fixture.root_path.join("objects").join(bucket);
        std::fs::create_dir(&path).unwrap();

        let first = fixture
            .objects
            .open_bucket(bucket, &digest, true, 1024)
            .unwrap();
        let identity = first.dir.identity().unwrap();
        drop(first);

        // The second call takes the successful existing-directory
        // `FILE_OPEN_IF` branch; its selected handle must pass validation.
        let second = fixture
            .objects
            .open_bucket(bucket, &digest, true, 1024)
            .unwrap();
        assert_eq!(second.dir.identity().unwrap(), identity);
        drop(second);

        std::fs::remove_dir(&path).unwrap();
        std::fs::write(&path, b"not a directory").unwrap();
        expect_error(
            fixture.objects.open_bucket(bucket, &digest, true, 1024),
            ArtifactError::ObjectParentNotDirectory,
        );
    }

    #[test]
    fn write_access_fallback_validates_the_reopened_handle() {
        let fixture = fixture();
        let bucket = "bb";
        let path = fixture.root_path.join("objects").join(bucket);
        std::fs::create_dir(&path).unwrap();
        let relocated = fixture.root_path.join("objects").join("bb-relocated");
        let replacement = path.clone();
        BUCKET_WRITE_REOPEN_HOOK.with(|slot| {
            *slot.borrow_mut() = Some(Box::new(move || {
                // Replace the bucket between the readable open and the
                // writable reopen used by the create fallback.
                std::fs::rename(&replacement, &relocated).unwrap();
                std::fs::write(&replacement, b"not a directory").unwrap();
            }));
        });

        expect_error(
            reopen_bucket_for_write(
                &fixture.objects,
                bucket,
                io::Error::other("forced FILE_OPEN_IF failure"),
            ),
            ArtifactError::ObjectParentNotDirectory,
        );
        assert!(BUCKET_WRITE_REOPEN_HOOK.with(|slot| slot.borrow().is_none()));
    }

    #[test]
    #[ignore = "requires Windows Developer Mode or elevation to create a directory symlink"]
    fn write_access_fallback_rejects_a_symlink_replacement() {
        let fixture = fixture();
        let bucket = "cc";
        let path = fixture.root_path.join("objects").join(bucket);
        std::fs::create_dir(&path).unwrap();
        let relocated = fixture.root_path.join("objects").join("cc-relocated");
        let outside = fixture.temp.path().join("outside");
        std::fs::create_dir(&outside).unwrap();
        let replacement = path.clone();
        BUCKET_WRITE_REOPEN_HOOK.with(|slot| {
            *slot.borrow_mut() = Some(Box::new(move || {
                std::fs::rename(&replacement, &relocated).unwrap();
                std::os::windows::fs::symlink_dir(&outside, &replacement).unwrap();
            }));
        });

        expect_error(
            reopen_bucket_for_write(
                &fixture.objects,
                bucket,
                io::Error::other("forced FILE_OPEN_IF failure"),
            ),
            ArtifactError::ObjectPathSymlink,
        );
    }

    #[test]
    fn publish_refuses_a_bucket_renamed_while_the_handle_is_held() {
        let fixture = fixture();
        let payload = b"held bucket rename";
        let digest = crate::sha256_hex(payload);
        let bucket_name = &digest[..2];
        let bucket = fixture
            .objects
            .open_bucket(bucket_name, &digest, true, 1024)
            .unwrap();
        let path = fixture.root_path.join("objects").join(bucket_name);
        let relocated = fixture.root_path.join("objects").join("held-relocated");
        let cwd_target = std::env::current_dir().unwrap().join(&digest);
        assert!(!cwd_target.exists());
        std::fs::rename(&path, &relocated).unwrap();

        match bucket.publish(&digest, payload) {
            Err(ArtifactError::PublicationFailed) => {}
            other => panic!("unexpected result: {other:?}"),
        }
        assert!(
            !cwd_target.exists(),
            "the rename target must be the opened bucket handle, not the process cwd"
        );
        let leftovers: Vec<_> = std::fs::read_dir(&relocated)
            .unwrap()
            .flatten()
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".artifact-")
            })
            .collect();
        assert!(
            leftovers.is_empty(),
            "publication must not leave temporary files"
        );
    }

    #[test]
    fn rename_information_layout_matches_fileapi() {
        assert_eq!(
            std::mem::offset_of!(FILE_RENAME_INFO, FileName),
            std::mem::offset_of!(FILE_RENAME_INFO, RootDirectory)
                + std::mem::size_of::<HANDLE>()
                + std::mem::size_of::<u32>()
        );
        assert!(
            std::mem::size_of::<FILE_RENAME_INFO>()
                >= std::mem::offset_of!(FILE_RENAME_INFO, FileName) + std::mem::size_of::<u16>()
        );
    }
}
