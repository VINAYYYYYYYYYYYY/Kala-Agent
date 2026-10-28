//! Owner-only file ACLs on Windows.
//!
//! This is the Windows counterpart of the `0o600` file mode and the `0o700`
//! directory mode. A file created in a directory whose inherited ACL names
//! other principals — a redirected or roaming profile, a OneDrive-synced
//! folder, a shared drive — is readable by them, so the token and settings
//! stores that rely on a POSIX mode need an explicit discretionary access
//! control list (DACL) here.
//!
//! [`restrict_to_current_user`] installs a *protected* DACL that grants full
//! control to the object's owner, `LocalSystem` and the built-in
//! `Administrators` group only, and removes every inherited entry. The object
//! is created by the process that calls this, so "owner" is the current user;
//! `Administrators` can take ownership of any file regardless of its DACL, so
//! denying it would claim more than Windows can deliver.

use std::io;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use std::ptr;

use windows_sys::Win32::Foundation::{ERROR_SUCCESS, LocalFree};
use windows_sys::Win32::Security::Authorization::{
    ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1, SE_FILE_OBJECT,
    SetNamedSecurityInfoW,
};
use windows_sys::Win32::Security::{
    ACL, DACL_SECURITY_INFORMATION, GetSecurityDescriptorDacl, PROTECTED_DACL_SECURITY_INFORMATION,
    PSECURITY_DESCRIPTOR,
};

/// The owner-only DACL in SDDL form.
///
/// `P` marks the DACL protected (no inherited entries are added), `OW` is the
/// owner-rights SID, `SY` is `LocalSystem`, `BA` is the built-in
/// `Administrators` group and `FA` is `FILE_ALL_ACCESS`.
const OWNER_ONLY_DACL: &str = "D:P(A;;FA;;;OW)(A;;FA;;;SY)(A;;FA;;;BA)";

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Restrict one file or directory to the current user with an explicit,
/// inheritance-free DACL.
///
/// The object must already exist. A failure leaves the previous ACL in place
/// and is reported to the caller: a secret written without its ACL is a
/// failure, not a warning.
///
/// # Errors
///
/// Returns the `Win32` error when the security descriptor cannot be built or
/// the DACL cannot be written.
pub fn restrict_to_current_user(path: &Path) -> io::Result<()> {
    let path_wide: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let mut sddl = wide(OWNER_ONLY_DACL);
    let mut descriptor: PSECURITY_DESCRIPTOR = ptr::null_mut();
    // SAFETY: both pointers name live, NUL-terminated UTF-16 buffers that
    // outlive the call, and `descriptor` receives an owned local allocation
    // that is released with `LocalFree` below.
    let converted = unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_mut_ptr(),
            SDDL_REVISION_1,
            &mut descriptor,
            ptr::null_mut(),
        )
    };
    if converted == 0 {
        return Err(io::Error::last_os_error());
    }
    let mut present = 0;
    let mut dacl: *mut ACL = ptr::null_mut();
    let mut defaulted = 0;
    // SAFETY: `descriptor` is a valid absolute security descriptor owned by
    // this frame; the out-parameters are live locals.
    let read =
        unsafe { GetSecurityDescriptorDacl(descriptor, &mut present, &mut dacl, &mut defaulted) };
    if read == 0 || present == 0 || defaulted != 0 {
        let error = if read == 0 {
            io::Error::last_os_error()
        } else {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "the owner-only security descriptor carries no explicit DACL",
            )
        };
        // SAFETY: `descriptor` came from the successful conversion above and
        // is released exactly once.
        unsafe {
            LocalFree(descriptor);
        }
        return Err(error);
    }
    // SAFETY: the path and the DACL both outlive the call, and the DACL
    // belongs to the descriptor that is released immediately afterwards.
    let status = unsafe {
        SetNamedSecurityInfoW(
            path_wide.as_ptr(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            ptr::null_mut(),
            ptr::null_mut(),
            dacl,
            ptr::null(),
        )
    };
    // SAFETY: the DACL above is only borrowed, so the descriptor is released
    // exactly once, after the call that read from it.
    unsafe {
        LocalFree(descriptor);
    }
    if status != ERROR_SUCCESS {
        return Err(io::Error::from_raw_os_error(status as i32));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::os::windows::ffi::OsStrExt;
    use std::ptr;

    use windows_sys::Win32::Foundation::{ERROR_SUCCESS, LocalFree};
    use windows_sys::Win32::Security::Authorization::{GetNamedSecurityInfoW, SE_FILE_OBJECT};
    use windows_sys::Win32::Security::{
        ACE_HEADER, ACL, ACL_SIZE_INFORMATION, AclSizeInformation, DACL_SECURITY_INFORMATION,
        GetAce, GetAclInformation, GetSecurityDescriptorControl, INHERITED_ACE,
        PSECURITY_DESCRIPTOR, SE_DACL_PROTECTED,
    };

    use super::restrict_to_current_user;

    /// The control flags, ACE count and per-ACE flags of a file's DACL.
    fn stored_dacl(path: &std::path::Path) -> (u16, Vec<u8>) {
        let mut wide: Vec<u16> = path
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        let mut dacl: *mut ACL = ptr::null_mut();
        let mut descriptor: PSECURITY_DESCRIPTOR = ptr::null_mut();
        // SAFETY: the path is NUL-terminated UTF-16 and the out-parameters are
        // live locals; the returned descriptor is released below.
        let status = unsafe {
            GetNamedSecurityInfoW(
                wide.as_mut_ptr(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION,
                ptr::null_mut(),
                ptr::null_mut(),
                &mut dacl,
                ptr::null_mut(),
                &mut descriptor,
            )
        };
        assert_eq!(status, ERROR_SUCCESS, "read the stored DACL");
        let mut control: u16 = 0;
        let mut revision: u32 = 0;
        // SAFETY: `descriptor` is the descriptor `GetNamedSecurityInfoW` just
        // returned, and the out-parameters are live locals.
        let ok = unsafe { GetSecurityDescriptorControl(descriptor, &mut control, &mut revision) };
        assert_ne!(ok, 0, "read the descriptor control flags");
        let mut size = ACL_SIZE_INFORMATION {
            AceCount: 0,
            AclBytesInUse: 0,
            AclBytesFree: 0,
        };
        // SAFETY: `dacl` points into `descriptor`, and `size` is a live local
        // of the exact shape `AclSizeInformation` writes.
        let ok = unsafe {
            GetAclInformation(
                dacl,
                (&raw mut size).cast(),
                std::mem::size_of::<ACL_SIZE_INFORMATION>() as u32,
                AclSizeInformation,
            )
        };
        assert_ne!(ok, 0, "read the DACL size");
        let mut flags = Vec::new();
        for index in 0..size.AceCount {
            let mut ace: *mut std::ffi::c_void = ptr::null_mut();
            // SAFETY: `index` is inside the ACE count just read, and `ace`
            // receives a pointer into `descriptor`.
            let ok = unsafe { GetAce(dacl, index, &mut ace) };
            assert_ne!(ok, 0, "read ACE {index}");
            // SAFETY: every ACE begins with an `ACE_HEADER`, and `ace` points
            // at one inside the descriptor that is still alive.
            let header = unsafe { ptr::read_unaligned(ace.cast::<ACE_HEADER>()) };
            flags.push(header.AceFlags);
        }
        // SAFETY: `descriptor` came from `GetNamedSecurityInfoW` and is
        // released exactly once.
        unsafe {
            LocalFree(descriptor);
        }
        (control, flags)
    }

    #[test]
    fn the_dacl_grants_the_owner_only_and_inherits_nothing() {
        let directory = tempfile::tempdir().expect("temp dir");
        let path = directory.path().join("secret.txt");
        fs::write(&path, b"token").expect("write the secret");

        restrict_to_current_user(&path).expect("restrict the secret");

        // The stored DACL is protected (no inherited entries) and holds the
        // three explicit owner/system/administrators grants.
        let (control, flags) = stored_dacl(&path);
        assert_ne!(control & SE_DACL_PROTECTED, 0, "the DACL is protected");
        assert_eq!(flags.len(), 3, "owner, LocalSystem and Administrators");
        assert!(
            flags.iter().all(|flags| flags & INHERITED_ACE as u8 == 0),
            "no ACE is inherited"
        );

        // The current user keeps read and write access.
        assert_eq!(fs::read(&path).expect("read the secret"), b"token");
        fs::write(&path, b"rotated").expect("rewrite the secret");
    }

    #[test]
    fn a_directory_is_restricted_too() {
        let directory = tempfile::tempdir().expect("temp dir");
        let nested = directory.path().join("studio");
        fs::create_dir(&nested).expect("create the nested directory");

        restrict_to_current_user(&nested).expect("restrict the directory");

        let (control, flags) = stored_dacl(&nested);
        assert_ne!(control & SE_DACL_PROTECTED, 0);
        assert_eq!(flags.len(), 3);
    }
}
