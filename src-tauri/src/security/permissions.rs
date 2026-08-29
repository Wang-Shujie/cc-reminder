use std::path::Path;

use crate::error::{AppError, ErrorDomain};

#[cfg(unix)]
pub fn ensure_private_directory(path: &Path) -> Result<(), AppError> {
    use std::os::unix::fs::PermissionsExt;

    std::fs::create_dir_all(path).map_err(|_| permission_error())?;
    let mode = std::fs::metadata(path)
        .map_err(|_| permission_error())?
        .permissions()
        .mode()
        & 0o777;
    if mode == 0o700 {
        return Ok(());
    }
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
        .map_err(|_| permission_error())
}

#[cfg(unix)]
pub fn ensure_private_file(path: &Path) -> Result<(), AppError> {
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

    std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .mode(0o600)
        .open(path)
        .map_err(|_| permission_error())?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
        .map_err(|_| permission_error())
}

#[cfg(not(unix))]
pub fn ensure_private_directory(path: &Path) -> Result<(), AppError> {
    std::fs::create_dir_all(path).map_err(|_| permission_error())
}

#[cfg(not(unix))]
pub fn ensure_private_file(path: &Path) -> Result<(), AppError> {
    std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(path)
        .map(|_| ())
        .map_err(|_| permission_error())
}

#[cfg(not(windows))]
pub fn ensure_current_user_dacl(path: &Path) -> Result<(), AppError> {
    if path.is_dir() {
        ensure_private_directory(path)
    } else {
        ensure_private_file(path)
    }
}

#[cfg(unix)]
pub fn validate_private_file(path: &Path) -> Result<(), AppError> {
    let symlink = std::fs::symlink_metadata(path).map_err(|_| permission_error())?;
    let metadata = std::fs::metadata(path).map_err(|_| permission_error())?;
    if symlink.file_type().is_symlink() || !private_file_owned_by(&metadata, effective_uid()) {
        return Err(permission_error());
    }
    Ok(())
}

#[cfg(unix)]
fn private_file_owned_by(metadata: &std::fs::Metadata, effective_uid: u32) -> bool {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    metadata.is_file()
        && metadata.uid() == effective_uid
        && metadata.permissions().mode() & 0o077 == 0
}

#[cfg(unix)]
fn effective_uid() -> u32 {
    unsafe extern "C" {
        fn geteuid() -> u32;
    }
    unsafe { geteuid() }
}

#[cfg(windows)]
pub fn validate_private_file(path: &Path) -> Result<(), AppError> {
    use std::os::windows::fs::MetadataExt;

    let metadata = std::fs::symlink_metadata(path).map_err(|_| permission_error())?;
    if is_reparse_point(metadata.file_attributes())
        || !metadata.is_file()
        || !windows_permissions::verify_current_user_dacl(path)?
    {
        return Err(permission_error());
    }
    Ok(())
}

#[cfg(windows)]
fn is_reparse_point(attributes: u32) -> bool {
    attributes & windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(windows)]
pub use windows_permissions::ensure_current_user_dacl;
#[cfg(windows)]
pub(crate) use windows_permissions::{
    NamedPipeSecurity, current_user_sid_string, verify_named_pipe_dacl,
};

#[cfg(all(test, windows))]
use windows_permissions::verify_current_user_dacl;

fn permission_error() -> AppError {
    AppError {
        domain: ErrorDomain::Storage,
        code: "storage.permission_failed".to_owned(),
        message: "private filesystem permissions could not be applied".to_owned(),
        suggested_action: None,
    }
}

#[cfg(windows)]
mod windows_permissions {
    use std::ffi::c_void;
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;
    use std::ptr::{null, null_mut};

    use windows_sys::Win32::Foundation::{CloseHandle, ERROR_SUCCESS, HANDLE, LocalFree};
    use windows_sys::Win32::Security::Authorization::{
        ConvertSidToStringSidW, EXPLICIT_ACCESS_W, GetSecurityInfo, SE_FILE_OBJECT,
        SE_KERNEL_OBJECT, SET_ACCESS, SetEntriesInAclW, SetNamedSecurityInfoW, TRUSTEE_IS_SID,
        TRUSTEE_IS_USER, TRUSTEE_IS_WELL_KNOWN_GROUP, TRUSTEE_W,
    };
    use windows_sys::Win32::Security::{
        ACCESS_ALLOWED_ACE, ACL, ACL_SIZE_INFORMATION, AclSizeInformation, CreateWellKnownSid,
        DACL_SECURITY_INFORMATION, EqualSid, GetAce, GetAclInformation, GetLengthSid,
        GetTokenInformation, InitializeSecurityDescriptor, NO_INHERITANCE, OBJECT_INHERIT_ACE,
        PROTECTED_DACL_SECURITY_INFORMATION, PSID, SECURITY_ATTRIBUTES, SECURITY_DESCRIPTOR,
        SECURITY_MAX_SID_SIZE, SetSecurityDescriptorDacl, TOKEN_QUERY, TOKEN_USER, TokenUser,
        WinBuiltinUsersSid, WinLocalSystemSid, WinWorldSid,
    };
    use windows_sys::Win32::Storage::FileSystem::{FILE_ALL_ACCESS, FILE_GENERIC_WRITE};
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    use super::{AppError, ensure_private_file, permission_error};

    const ACCESS_ALLOWED_ACE_TYPE: u8 = 0;

    pub(crate) struct NamedPipeSecurity {
        acl: *mut ACL,
        descriptor: Box<SECURITY_DESCRIPTOR>,
        attributes: SECURITY_ATTRIBUTES,
    }

    impl NamedPipeSecurity {
        pub(crate) fn new() -> Result<Self, AppError> {
            let current_user = current_user_sid()?;
            let local_system = well_known_sid(WinLocalSystemSid)?;
            let entries = [
                explicit_access(&current_user, TRUSTEE_IS_USER, NO_INHERITANCE),
                explicit_access(&local_system, TRUSTEE_IS_WELL_KNOWN_GROUP, NO_INHERITANCE),
            ];
            let mut acl: *mut ACL = null_mut();
            let status = unsafe {
                SetEntriesInAclW(entries.len() as u32, entries.as_ptr(), null(), &mut acl)
            };
            if status != ERROR_SUCCESS || acl.is_null() {
                return Err(permission_error());
            }
            let mut descriptor = Box::<SECURITY_DESCRIPTOR>::default();
            let descriptor_pointer = (&mut *descriptor as *mut SECURITY_DESCRIPTOR).cast();
            if unsafe { InitializeSecurityDescriptor(descriptor_pointer, 1) } == 0
                || unsafe { SetSecurityDescriptorDacl(descriptor_pointer, 1, acl, 0) } == 0
            {
                unsafe { LocalFree(acl.cast()) };
                return Err(permission_error());
            }
            let attributes = SECURITY_ATTRIBUTES {
                nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
                lpSecurityDescriptor: (&mut *descriptor as *mut SECURITY_DESCRIPTOR).cast(),
                bInheritHandle: 0,
            };
            Ok(Self {
                acl,
                descriptor,
                attributes,
            })
        }

        pub(crate) fn attributes(&self) -> *const SECURITY_ATTRIBUTES {
            let _ = &self.descriptor;
            &self.attributes
        }
    }

    impl Drop for NamedPipeSecurity {
        fn drop(&mut self) {
            unsafe { LocalFree(self.acl.cast()) };
        }
    }

    pub(crate) fn current_user_sid_string() -> Result<String, AppError> {
        let sid = current_user_sid()?;
        let mut string = null_mut();
        if unsafe { ConvertSidToStringSidW(sid.as_ptr().cast_mut().cast(), &mut string) } == 0
            || string.is_null()
        {
            return Err(permission_error());
        }
        let length = (0_usize..)
            .find(|&index| unsafe { *string.add(index) } == 0)
            .ok_or_else(permission_error)?;
        let value = String::from_utf16(unsafe { std::slice::from_raw_parts(string, length) })
            .map_err(|_| permission_error());
        unsafe { LocalFree(string.cast()) };
        value
    }

    pub(crate) fn verify_named_pipe_dacl(handle: HANDLE) -> Result<bool, AppError> {
        let current_user = current_user_sid()?;
        let local_system = well_known_sid(WinLocalSystemSid)?;
        let world = well_known_sid(WinWorldSid)?;
        let users = well_known_sid(WinBuiltinUsersSid)?;
        let mut acl: *mut ACL = null_mut();
        let mut descriptor = null_mut();
        let status = unsafe {
            GetSecurityInfo(
                handle,
                SE_KERNEL_OBJECT,
                DACL_SECURITY_INFORMATION,
                null_mut(),
                null_mut(),
                &mut acl,
                null_mut(),
                &mut descriptor,
            )
        };
        if status != ERROR_SUCCESS || acl.is_null() || descriptor.is_null() {
            return Err(permission_error());
        }
        let verified = verify_acl(acl, &current_user, &local_system, &world, &users);
        unsafe { LocalFree(descriptor) };
        verified
    }

    pub fn ensure_current_user_dacl(path: &Path) -> Result<(), AppError> {
        if !path.exists() {
            ensure_private_file(path)?;
        }
        let current_user = current_user_sid()?;
        let local_system = well_known_sid(WinLocalSystemSid)?;
        let inheritance = if path.is_dir() {
            OBJECT_INHERIT_ACE | windows_sys::Win32::Security::CONTAINER_INHERIT_ACE
        } else {
            NO_INHERITANCE
        };
        let entries = [
            explicit_access(&current_user, TRUSTEE_IS_USER, inheritance),
            explicit_access(&local_system, TRUSTEE_IS_WELL_KNOWN_GROUP, inheritance),
        ];
        let mut acl: *mut ACL = null_mut();
        let status =
            unsafe { SetEntriesInAclW(entries.len() as u32, entries.as_ptr(), null(), &mut acl) };
        if status != ERROR_SUCCESS || acl.is_null() {
            return Err(permission_error());
        }
        let wide_path = wide_path(path);
        let status = unsafe {
            SetNamedSecurityInfoW(
                wide_path.as_ptr(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
                null_mut(),
                null_mut(),
                acl,
                null(),
            )
        };
        unsafe {
            LocalFree(acl.cast());
        }
        if status != ERROR_SUCCESS || !verify_current_user_dacl(path)? {
            return Err(permission_error());
        }
        Ok(())
    }

    pub(super) fn verify_current_user_dacl(path: &Path) -> Result<bool, AppError> {
        use windows_sys::Win32::Security::Authorization::GetNamedSecurityInfoW;

        let current_user = current_user_sid()?;
        let local_system = well_known_sid(WinLocalSystemSid)?;
        let world = well_known_sid(WinWorldSid)?;
        let users = well_known_sid(WinBuiltinUsersSid)?;
        let wide_path = wide_path(path);
        let mut acl: *mut ACL = null_mut();
        let mut descriptor = null_mut();
        let status = unsafe {
            GetNamedSecurityInfoW(
                wide_path.as_ptr(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION,
                null_mut(),
                null_mut(),
                &mut acl,
                null_mut(),
                &mut descriptor,
            )
        };
        if status != ERROR_SUCCESS || acl.is_null() || descriptor.is_null() {
            return Err(permission_error());
        }
        let verified = verify_acl(acl, &current_user, &local_system, &world, &users);
        unsafe {
            LocalFree(descriptor);
        }
        verified
    }

    fn verify_acl(
        acl: *const ACL,
        current_user: &[u8],
        local_system: &[u8],
        world: &[u8],
        users: &[u8],
    ) -> Result<bool, AppError> {
        let mut info = ACL_SIZE_INFORMATION::default();
        if unsafe {
            GetAclInformation(
                acl,
                (&mut info as *mut ACL_SIZE_INFORMATION).cast(),
                std::mem::size_of::<ACL_SIZE_INFORMATION>() as u32,
                AclSizeInformation,
            )
        } == 0
        {
            return Err(permission_error());
        }
        let mut has_user = false;
        let mut has_system = false;
        for index in 0..info.AceCount {
            let mut ace_pointer: *mut c_void = null_mut();
            if unsafe { GetAce(acl, index, &mut ace_pointer) } == 0 || ace_pointer.is_null() {
                return Err(permission_error());
            }
            let ace = unsafe { &*(ace_pointer.cast::<ACCESS_ALLOWED_ACE>()) };
            if ace.Header.AceType != ACCESS_ALLOWED_ACE_TYPE {
                continue;
            }
            let sid = (&ace.SidStart as *const u32).cast_mut().cast();
            let is_user = equal_sid(sid, current_user);
            let is_system = equal_sid(sid, local_system);
            has_user |= is_user;
            has_system |= is_system;
            let broad = equal_sid(sid, world) || equal_sid(sid, users);
            if broad && ace.Mask & FILE_GENERIC_WRITE != 0 {
                return Ok(false);
            }
            if !is_user && !is_system && ace.Mask & FILE_GENERIC_WRITE != 0 {
                return Ok(false);
            }
        }
        Ok(has_user && has_system)
    }

    fn explicit_access(sid: &[u8], trustee_type: i32, inheritance: u32) -> EXPLICIT_ACCESS_W {
        EXPLICIT_ACCESS_W {
            grfAccessPermissions: FILE_ALL_ACCESS,
            grfAccessMode: SET_ACCESS,
            grfInheritance: inheritance,
            Trustee: TRUSTEE_W {
                pMultipleTrustee: null_mut(),
                MultipleTrusteeOperation: 0,
                TrusteeForm: TRUSTEE_IS_SID,
                TrusteeType: trustee_type,
                ptstrName: sid.as_ptr().cast_mut().cast(),
            },
        }
    }

    fn current_user_sid() -> Result<Vec<u8>, AppError> {
        let mut token: HANDLE = null_mut();
        if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
            return Err(permission_error());
        }
        let result = (|| {
            let mut required = 0;
            unsafe {
                GetTokenInformation(token, TokenUser, null_mut(), 0, &mut required);
            }
            if required == 0 {
                return Err(permission_error());
            }
            let mut buffer = vec![0_u8; required as usize];
            if unsafe {
                GetTokenInformation(
                    token,
                    TokenUser,
                    buffer.as_mut_ptr().cast(),
                    required,
                    &mut required,
                )
            } == 0
            {
                return Err(permission_error());
            }
            let token_user = unsafe { &*(buffer.as_ptr().cast::<TOKEN_USER>()) };
            copy_sid(token_user.User.Sid)
        })();
        unsafe {
            CloseHandle(token);
        }
        result
    }

    fn well_known_sid(kind: i32) -> Result<Vec<u8>, AppError> {
        let mut size = SECURITY_MAX_SID_SIZE;
        let mut sid = vec![0_u8; size as usize];
        if unsafe { CreateWellKnownSid(kind, null_mut(), sid.as_mut_ptr().cast(), &mut size) } == 0
        {
            return Err(permission_error());
        }
        sid.truncate(size as usize);
        Ok(sid)
    }

    fn copy_sid(sid: PSID) -> Result<Vec<u8>, AppError> {
        let length = unsafe { GetLengthSid(sid) };
        if length == 0 {
            return Err(permission_error());
        }
        let mut copy = vec![0_u8; length as usize];
        unsafe {
            std::ptr::copy_nonoverlapping(sid.cast::<u8>(), copy.as_mut_ptr(), length as usize);
        }
        Ok(copy)
    }

    fn equal_sid(left: PSID, right: &[u8]) -> bool {
        unsafe { EqualSid(left, right.as_ptr().cast_mut().cast()) != 0 }
    }

    fn wide_path(path: &Path) -> Vec<u16> {
        path.as_os_str().encode_wide().chain(Some(0)).collect()
    }
}

#[cfg(all(test, unix))]
mod unix_tests {
    use std::os::unix::fs::PermissionsExt;

    use tempfile::tempdir;

    use super::{ensure_private_directory, ensure_private_file, private_file_owned_by};

    #[test]
    fn private_paths_apply_owner_only_unix_modes() {
        let root = tempdir().unwrap();
        let directory = root.path().join("private");
        let file = directory.join("data.sqlite3");

        ensure_private_directory(&directory).unwrap();
        ensure_private_file(&file).unwrap();

        assert_eq!(
            std::fs::metadata(&directory).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            std::fs::metadata(&file).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    #[test]
    fn private_cache_metadata_rejects_a_different_effective_user() {
        use std::os::unix::fs::MetadataExt;

        let root = tempdir().unwrap();
        let file = root.path().join("cache.json");
        ensure_private_file(&file).unwrap();
        let metadata = std::fs::metadata(file).unwrap();

        assert!(private_file_owned_by(&metadata, metadata.uid()));
        assert!(!private_file_owned_by(
            &metadata,
            metadata.uid().wrapping_add(1)
        ));
    }
}

#[cfg(all(test, windows))]
mod windows_tests {
    use tempfile::tempdir;

    use super::{ensure_current_user_dacl, is_reparse_point, verify_current_user_dacl};
    use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;

    #[test]
    fn private_file_dacl_has_no_broad_write_principal() {
        let root = tempdir().unwrap();
        let file = root.path().join("data.sqlite3");

        ensure_current_user_dacl(&file).unwrap();

        assert!(verify_current_user_dacl(&file).unwrap());
    }

    #[test]
    fn cache_validation_rejects_reparse_point_attributes() {
        assert!(!is_reparse_point(0));
        assert!(is_reparse_point(FILE_ATTRIBUTE_REPARSE_POINT));
    }

    #[test]
    fn private_cache_validation_rejects_a_real_file_symlink() {
        let root = tempdir().unwrap();
        let target = root.path().join("target.json");
        let link = root.path().join("cache.json");
        std::fs::write(&target, b"{}").unwrap();
        ensure_current_user_dacl(&target).unwrap();
        match std::os::windows::fs::symlink_file(&target, &link) {
            Ok(()) => assert!(super::validate_private_file(&link).is_err()),
            Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {}
            Err(error) => panic!("failed to create test symlink: {error}"),
        }
    }
}
