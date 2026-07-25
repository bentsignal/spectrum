use std::{ffi::c_void, os::windows::ffi::OsStrExt, os::windows::io::AsRawHandle, path::Path, ptr};

use windows_sys::Win32::{
    Foundation::{CloseHandle, GENERIC_ALL, HANDLE, LocalFree},
    Security::{
        ACCESS_ALLOWED_ACE, ACL, ACL_SIZE_INFORMATION, AclSizeInformation,
        Authorization::{
            ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
            GetSecurityInfo, SE_FILE_OBJECT,
        },
        DACL_SECURITY_INFORMATION, EqualSid, GetAce, GetAclInformation, GetFileSecurityW,
        GetSecurityDescriptorControl, GetSecurityDescriptorDacl, GetSecurityDescriptorOwner,
        GetTokenInformation, OWNER_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID,
        SE_DACL_PROTECTED, SetFileSecurityW, TOKEN_QUERY, TOKEN_USER, TokenUser,
    },
    Storage::FileSystem::FILE_ALL_ACCESS,
    System::{
        SystemServices::{ACCESS_ALLOWED_ACE_TYPE, SECURITY_DESCRIPTOR_REVISION},
        Threading::{GetCurrentProcess, OpenProcessToken},
    },
};

use crate::{BridgeError, BridgeResult};

pub(crate) struct OwnedSecurityDescriptor(PSECURITY_DESCRIPTOR);

unsafe impl Send for OwnedSecurityDescriptor {}
unsafe impl Sync for OwnedSecurityDescriptor {}

impl OwnedSecurityDescriptor {
    pub(crate) fn as_ptr(&self) -> PSECURITY_DESCRIPTOR {
        self.0
    }
}

impl Drop for OwnedSecurityDescriptor {
    fn drop(&mut self) {
        unsafe {
            LocalFree(self.0);
        }
    }
}

pub(crate) fn current_user_only_descriptor() -> BridgeResult<OwnedSecurityDescriptor> {
    let sid = current_user_sid_string()?;
    descriptor_from_sddl(&format!("O:{sid}D:P(A;;GA;;;{sid})"))
}

pub(crate) fn apply_private_acl(path: &Path) -> BridgeResult<()> {
    let descriptor = current_user_only_descriptor()?;
    apply_security_descriptor(path, &descriptor, descriptor.as_ptr())?;
    verify_private_acl(path)
}

fn apply_security_descriptor(
    path: &Path,
    dacl_descriptor: &OwnedSecurityDescriptor,
    owner_descriptor: PSECURITY_DESCRIPTOR,
) -> BridgeResult<()> {
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    // Apply the protected current-user DACL first. Elevated Windows tokens can
    // create a file owned by the Administrators group; granting the current
    // logon SID full access gives the following owner update WRITE_OWNER
    // without enabling or depending on a process privilege.
    if unsafe {
        SetFileSecurityW(
            wide.as_ptr(),
            DACL_SECURITY_INFORMATION,
            dacl_descriptor.as_ptr(),
        )
    } == 0
    {
        return Err(std::io::Error::last_os_error().into());
    }
    if unsafe { SetFileSecurityW(wide.as_ptr(), OWNER_SECURITY_INFORMATION, owner_descriptor) } == 0
    {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(())
}

pub(crate) fn verify_private_acl(path: &Path) -> BridgeResult<()> {
    let mut actual =
        read_security_descriptor(path, DACL_SECURITY_INFORMATION | OWNER_SECURITY_INFORMATION)?;
    verify_descriptor(actual.as_mut_ptr().cast())
}

fn read_security_descriptor(path: &Path, information: u32) -> BridgeResult<Vec<u8>> {
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut needed = 0;
    unsafe {
        GetFileSecurityW(
            wide.as_ptr(),
            information,
            ptr::null_mut(),
            0,
            &raw mut needed,
        );
    }
    if needed == 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    let mut actual = vec![0_u8; needed as usize];
    if unsafe {
        GetFileSecurityW(
            wide.as_ptr(),
            information,
            actual.as_mut_ptr().cast(),
            needed,
            &raw mut needed,
        )
    } == 0
    {
        return Err(std::io::Error::last_os_error().into());
    }

    Ok(actual)
}

pub(crate) fn verify_private_handle(file: &std::fs::File) -> BridgeResult<()> {
    let mut owner = ptr::null_mut();
    let mut dacl = ptr::null_mut();
    let mut descriptor = ptr::null_mut();
    let status = unsafe {
        GetSecurityInfo(
            file.as_raw_handle() as HANDLE,
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            &raw mut owner,
            ptr::null_mut(),
            &raw mut dacl,
            ptr::null_mut(),
            &raw mut descriptor,
        )
    };
    if status != 0 || descriptor.is_null() {
        return Err(std::io::Error::from_raw_os_error(status as i32).into());
    }
    let result = verify_descriptor(descriptor);
    unsafe {
        LocalFree(descriptor);
    }
    result
}

fn verify_descriptor(actual_descriptor: PSECURITY_DESCRIPTOR) -> BridgeResult<()> {
    let current = current_user_sid()?;
    let actual_owner = descriptor_owner(actual_descriptor)?;
    if unsafe { EqualSid(actual_owner, current.as_sid()) } == 0 {
        return Err(BridgeError::Authentication(
            "binding path owner is not the current logon SID".into(),
        ));
    }
    verify_descriptor_dacl(actual_descriptor, current.as_sid())
}

fn verify_descriptor_dacl(
    actual_descriptor: PSECURITY_DESCRIPTOR,
    current_sid: PSID,
) -> BridgeResult<()> {
    let mut control = 0;
    let mut revision = 0;
    if unsafe {
        GetSecurityDescriptorControl(actual_descriptor, &raw mut control, &raw mut revision)
    } == 0
        || control & SE_DACL_PROTECTED == 0
    {
        return Err(BridgeError::Authentication(
            "binding path DACL is not protected from inheritance".into(),
        ));
    }
    let mut present = 0;
    let mut defaulted = 0;
    let mut acl: *mut ACL = ptr::null_mut();
    if unsafe {
        GetSecurityDescriptorDacl(
            actual_descriptor,
            &raw mut present,
            &raw mut acl,
            &raw mut defaulted,
        )
    } == 0
        || present == 0
        || acl.is_null()
    {
        return Err(BridgeError::Authentication("private DACL is absent".into()));
    }
    let mut size = ACL_SIZE_INFORMATION {
        AceCount: 0,
        AclBytesInUse: 0,
        AclBytesFree: 0,
    };
    if unsafe {
        GetAclInformation(
            acl,
            (&raw mut size).cast::<c_void>(),
            std::mem::size_of::<ACL_SIZE_INFORMATION>() as u32,
            AclSizeInformation,
        )
    } == 0
    {
        return Err(std::io::Error::last_os_error().into());
    }
    if size.AceCount != 1 {
        return Err(BridgeError::Authentication(
            "binding path DACL must contain exactly one access rule".into(),
        ));
    }
    let mut raw_ace = ptr::null_mut();
    if unsafe { GetAce(acl, 0, &raw mut raw_ace) } == 0 || raw_ace.is_null() {
        return Err(std::io::Error::last_os_error().into());
    }
    let ace = unsafe { &*raw_ace.cast::<ACCESS_ALLOWED_ACE>() };
    if u32::from(ace.Header.AceType) != ACCESS_ALLOWED_ACE_TYPE {
        return Err(BridgeError::Authentication(
            "binding path DACL contains a non-allow rule".into(),
        ));
    }
    let allowed_sid = (&raw const ace.SidStart).cast_mut().cast::<c_void>();
    if unsafe { EqualSid(allowed_sid, current_sid) } == 0 {
        return Err(BridgeError::Authentication(
            "binding path DACL grants a principal other than the current user".into(),
        ));
    }
    if ace.Mask != GENERIC_ALL && ace.Mask & FILE_ALL_ACCESS != FILE_ALL_ACCESS {
        return Err(BridgeError::Authentication(
            "binding path DACL does not grant full current-user access".into(),
        ));
    }
    Ok(())
}

fn descriptor_owner(descriptor: PSECURITY_DESCRIPTOR) -> BridgeResult<PSID> {
    let mut owner = ptr::null_mut();
    let mut defaulted = 0;
    if unsafe { GetSecurityDescriptorOwner(descriptor, &raw mut owner, &raw mut defaulted) } == 0
        || owner.is_null()
    {
        return Err(BridgeError::Authentication(
            "security descriptor has no owner SID".into(),
        ));
    }
    Ok(owner)
}

fn descriptor_from_sddl(sddl: &str) -> BridgeResult<OwnedSecurityDescriptor> {
    let wide: Vec<u16> = sddl.encode_utf16().chain(Some(0)).collect();
    let mut descriptor = ptr::null_mut();
    if unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            wide.as_ptr(),
            SECURITY_DESCRIPTOR_REVISION,
            &raw mut descriptor,
            ptr::null_mut(),
        )
    } == 0
    {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(OwnedSecurityDescriptor(descriptor))
}

fn current_user_sid_string() -> BridgeResult<String> {
    let user = current_user_sid()?;
    let sid = user.as_sid();
    let mut text = ptr::null_mut();
    if unsafe { ConvertSidToStringSidW(sid, &raw mut text) } == 0 || text.is_null() {
        return Err(std::io::Error::last_os_error().into());
    }
    let length = unsafe {
        let mut length = 0;
        while *text.add(length) != 0 {
            length += 1;
        }
        length
    };
    let result = String::from_utf16(unsafe { std::slice::from_raw_parts(text, length) })
        .map_err(|_| BridgeError::Authentication("current SID is not valid UTF-16".into()));
    unsafe {
        LocalFree(text.cast());
    }
    result
}

struct OwnedTokenUser(Vec<u8>);

impl OwnedTokenUser {
    fn as_sid(&self) -> PSID {
        unsafe { (*(self.0.as_ptr().cast::<TOKEN_USER>())).User.Sid }
    }
}

fn current_user_sid() -> BridgeResult<OwnedTokenUser> {
    let token = process_token()?;
    token_user(token.0).map(OwnedTokenUser)
}

struct OwnedHandle(HANDLE);

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}

fn process_token() -> BridgeResult<OwnedHandle> {
    let mut token = ptr::null_mut();
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &raw mut token) } == 0 {
        Err(std::io::Error::last_os_error().into())
    } else {
        Ok(OwnedHandle(token))
    }
}

fn token_user(token: HANDLE) -> BridgeResult<Vec<u8>> {
    let mut needed = 0;
    unsafe {
        GetTokenInformation(token, TokenUser, ptr::null_mut(), 0, &raw mut needed);
    }
    if needed == 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    let mut bytes = vec![0_u8; needed as usize];
    if unsafe {
        GetTokenInformation(
            token,
            TokenUser,
            bytes.as_mut_ptr().cast::<c_void>(),
            needed,
            &raw mut needed,
        )
    } == 0
    {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use std::{fs, ptr};

    use super::*;

    #[test]
    fn canonical_full_access_forms_pass_but_extra_principal_fails() {
        let sid = current_user_sid_string().unwrap();
        let file_all = descriptor_from_sddl(&format!("O:{sid}D:P(A;;FA;;;{sid})")).unwrap();
        verify_descriptor(file_all.as_ptr()).unwrap();

        let broad =
            descriptor_from_sddl(&format!("O:{sid}D:P(A;;FA;;;{sid})(A;;GR;;;WD)")).unwrap();
        assert!(verify_descriptor(broad.as_ptr()).is_err());
    }

    #[test]
    fn two_phase_hardening_handles_inherited_admin_owned_objects() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("inherited.tmp");
        fs::write(&path, b"private").unwrap();

        let current = current_user_sid().unwrap();
        let mut before = read_security_descriptor(&path, OWNER_SECURITY_INFORMATION).unwrap();
        let owner = descriptor_owner(before.as_mut_ptr().cast()).unwrap();
        let began_with_different_owner = unsafe { EqualSid(owner, current.as_sid()) } == 0;
        if std::env::var_os("GITHUB_ACTIONS").is_some() {
            assert!(
                began_with_different_owner,
                "the elevated Windows runner should exercise inherited administrator ownership"
            );
        }

        apply_private_acl(&path).unwrap();
        verify_private_acl(&path).unwrap();
    }

    #[test]
    fn failed_owner_phase_leaves_a_fail_closed_current_user_dacl() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("owner-failure.tmp");
        fs::write(&path, b"private").unwrap();
        let descriptor = current_user_only_descriptor().unwrap();

        let error = apply_security_descriptor(&path, &descriptor, ptr::null_mut()).unwrap_err();
        assert!(matches!(error, BridgeError::Io(_)));

        let current = current_user_sid().unwrap();
        let mut after = read_security_descriptor(&path, DACL_SECURITY_INFORMATION).unwrap();
        verify_descriptor_dacl(after.as_mut_ptr().cast(), current.as_sid()).unwrap();
    }
}
