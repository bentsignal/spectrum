use std::{ffi::c_void, os::windows::ffi::OsStrExt, os::windows::io::AsRawHandle, path::Path, ptr};

use windows_sys::Win32::{
    Foundation::{CloseHandle, HANDLE, LocalFree},
    Security::{
        ACL, ACL_SIZE_INFORMATION, AclSizeInformation,
        Authorization::{
            ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
            GetSecurityInfo, SE_FILE_OBJECT,
        },
        DACL_SECURITY_INFORMATION, EqualSid, GetAclInformation, GetFileSecurityW,
        GetSecurityDescriptorDacl, GetSecurityDescriptorOwner, GetTokenInformation,
        OWNER_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID, SetFileSecurityW, TOKEN_QUERY,
        TOKEN_USER, TokenUser,
    },
    System::{
        SystemServices::SECURITY_DESCRIPTOR_REVISION,
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
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let applied = unsafe {
        SetFileSecurityW(
            wide.as_ptr(),
            DACL_SECURITY_INFORMATION,
            descriptor.as_ptr(),
        )
    };
    if applied == 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    verify_private_acl(path)
}

pub(crate) fn verify_private_acl(path: &Path) -> BridgeResult<()> {
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let information = DACL_SECURITY_INFORMATION | OWNER_SECURITY_INFORMATION;
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

    verify_descriptor(actual.as_mut_ptr().cast())
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
    let expected = current_user_only_descriptor()?;
    verify_owner_matches_current(actual_descriptor, expected.as_ptr())?;
    if dacl_bytes(actual_descriptor)? != dacl_bytes(expected.as_ptr())? {
        return Err(BridgeError::Authentication(
            "binding path does not have the exact current-user protected DACL".into(),
        ));
    }
    Ok(())
}

fn verify_owner_matches_current(
    actual: PSECURITY_DESCRIPTOR,
    expected: PSECURITY_DESCRIPTOR,
) -> BridgeResult<()> {
    let actual_owner = descriptor_owner(actual)?;
    let expected_owner = descriptor_owner(expected)?;
    if unsafe { EqualSid(actual_owner, expected_owner) } == 0 {
        return Err(BridgeError::Authentication(
            "binding path owner is not the current logon SID".into(),
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

fn dacl_bytes(descriptor: PSECURITY_DESCRIPTOR) -> BridgeResult<Vec<u8>> {
    let mut present = 0;
    let mut defaulted = 0;
    let mut acl: *mut ACL = ptr::null_mut();
    if unsafe {
        GetSecurityDescriptorDacl(
            descriptor,
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
    Ok(
        unsafe { std::slice::from_raw_parts(acl.cast::<u8>(), size.AclBytesInUse as usize) }
            .to_vec(),
    )
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
    let token = process_token()?;
    let user = token_user(token.0)?;
    let sid = unsafe { (*(user.as_ptr().cast::<TOKEN_USER>())).User.Sid };
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
