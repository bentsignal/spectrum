use std::{
    ffi::c_void,
    io::{Read, Write},
    ptr,
    sync::Mutex,
    time::Duration,
};

use windows_sys::Win32::{
    Foundation::{
        CloseHandle, DUPLICATE_SAME_ACCESS, DuplicateHandle, ERROR_PIPE_CONNECTED, GENERIC_READ,
        GENERIC_WRITE, GetLastError, HANDLE, INVALID_HANDLE_VALUE, LocalFree,
    },
    Security::{
        Authorization::ConvertStringSecurityDescriptorToSecurityDescriptorW, EqualSid,
        GetTokenInformation, PSECURITY_DESCRIPTOR, RevertToSelf, SECURITY_ATTRIBUTES, TOKEN_QUERY,
        TOKEN_USER, TokenUser,
    },
    Storage::FileSystem::{
        CreateFileW, FILE_FLAG_FIRST_PIPE_INSTANCE, FlushFileBuffers, OPEN_EXISTING,
        PIPE_ACCESS_DUPLEX, ReadFile, WriteFile,
    },
    System::{
        Pipes::{
            ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, ImpersonateNamedPipeClient,
            PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_WAIT,
        },
        SystemServices::SECURITY_DESCRIPTOR_REVISION,
        Threading::{GetCurrentProcess, GetCurrentThread, OpenProcessToken, OpenThreadToken},
    },
};

use crate::{BridgeError, BridgeResult, EndpointAddress};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PeerIdentity {
    pub same_user: bool,
}

pub struct LocalListener {
    name: Vec<u16>,
    pending: Mutex<HANDLE>,
    descriptor: PSECURITY_DESCRIPTOR,
}

unsafe impl Send for LocalListener {}
unsafe impl Sync for LocalListener {}

impl LocalListener {
    pub fn bind(address: &EndpointAddress) -> BridgeResult<Self> {
        let name = pipe_name(address)?;
        let descriptor = user_only_descriptor()?;
        let handle = create_pipe(&name, descriptor, true);
        if handle == INVALID_HANDLE_VALUE {
            unsafe {
                LocalFree(descriptor);
            }
            return Err(last_error().into());
        }
        Ok(Self {
            name,
            pending: Mutex::new(handle),
            descriptor,
        })
    }

    pub fn accept(&self) -> BridgeResult<(LocalStream, PeerIdentity)> {
        let mut pending = self.pending.lock().map_err(|_| BridgeError::Closed)?;
        let connected = unsafe { ConnectNamedPipe(*pending, ptr::null_mut()) };
        if connected == 0 && unsafe { GetLastError() } != ERROR_PIPE_CONNECTED {
            return Err(last_error().into());
        }
        let same_user = same_user_client(*pending)?;
        if !same_user {
            unsafe {
                DisconnectNamedPipe(*pending);
            }
            return Err(BridgeError::Authentication(
                "named-pipe client belongs to another user".into(),
            ));
        }
        let accepted = *pending;
        let replacement = create_pipe(&self.name, self.descriptor, false);
        if replacement == INVALID_HANDLE_VALUE {
            unsafe {
                DisconnectNamedPipe(accepted);
                CloseHandle(accepted);
            }
            return Err(last_error().into());
        }
        *pending = replacement;
        Ok((LocalStream { handle: accepted }, PeerIdentity { same_user }))
    }

    pub fn set_nonblocking(&self, _nonblocking: bool) -> BridgeResult<()> {
        Ok(())
    }
}

impl Drop for LocalListener {
    fn drop(&mut self) {
        if let Ok(handle) = self.pending.lock() {
            unsafe {
                CloseHandle(*handle);
            }
        }
        unsafe {
            LocalFree(self.descriptor);
        }
    }
}

pub struct LocalStream {
    handle: HANDLE,
}

unsafe impl Send for LocalStream {}

impl LocalStream {
    pub fn connect(address: &EndpointAddress) -> BridgeResult<Self> {
        let name = pipe_name(address)?;
        let handle = unsafe {
            CreateFileW(
                name.as_ptr(),
                GENERIC_READ | GENERIC_WRITE,
                0,
                ptr::null(),
                OPEN_EXISTING,
                0,
                ptr::null_mut(),
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            return Err(last_error().into());
        }
        Ok(Self { handle })
    }

    pub fn try_clone(&self) -> BridgeResult<Self> {
        let process = unsafe { GetCurrentProcess() };
        let mut duplicate = ptr::null_mut();
        let result = unsafe {
            DuplicateHandle(
                process,
                self.handle,
                process,
                &raw mut duplicate,
                0,
                0,
                DUPLICATE_SAME_ACCESS,
            )
        };
        if result == 0 {
            return Err(last_error().into());
        }
        Ok(Self { handle: duplicate })
    }

    pub fn set_read_timeout(&self, _timeout: Option<Duration>) -> BridgeResult<()> {
        Ok(())
    }

    pub fn set_write_timeout(&self, _timeout: Option<Duration>) -> BridgeResult<()> {
        Ok(())
    }
}

impl Read for LocalStream {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        let length = u32::try_from(buffer.len()).unwrap_or(u32::MAX);
        let mut read = 0;
        let result = unsafe {
            ReadFile(
                self.handle,
                buffer.as_mut_ptr(),
                length,
                &raw mut read,
                ptr::null_mut(),
            )
        };
        if result == 0 {
            Err(last_error())
        } else {
            Ok(read as usize)
        }
    }
}

impl Write for LocalStream {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        let length = u32::try_from(buffer.len()).unwrap_or(u32::MAX);
        let mut written = 0;
        let result = unsafe {
            WriteFile(
                self.handle,
                buffer.as_ptr(),
                length,
                &raw mut written,
                ptr::null_mut(),
            )
        };
        if result == 0 {
            Err(last_error())
        } else {
            Ok(written as usize)
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        if unsafe { FlushFileBuffers(self.handle) } == 0 {
            Err(last_error())
        } else {
            Ok(())
        }
    }
}

impl Drop for LocalStream {
    fn drop(&mut self) {
        unsafe {
            DisconnectNamedPipe(self.handle);
            CloseHandle(self.handle);
        }
    }
}

fn create_pipe(name: &[u16], descriptor: PSECURITY_DESCRIPTOR, first: bool) -> HANDLE {
    let mut attributes = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor,
        bInheritHandle: 0,
    };
    let first_flag = if first {
        FILE_FLAG_FIRST_PIPE_INSTANCE
    } else {
        0
    };
    unsafe {
        CreateNamedPipeW(
            name.as_ptr(),
            PIPE_ACCESS_DUPLEX | first_flag,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            crate::MAX_AUTHENTICATED_CONNECTIONS as u32,
            64 * 1024,
            64 * 1024,
            0,
            &raw mut attributes,
        )
    }
}

fn user_only_descriptor() -> BridgeResult<PSECURITY_DESCRIPTOR> {
    let sddl: Vec<u16> = "D:P(A;;GA;;;OW)\0".encode_utf16().collect();
    let mut descriptor = ptr::null_mut();
    let converted = unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            SECURITY_DESCRIPTOR_REVISION,
            &raw mut descriptor,
            ptr::null_mut(),
        )
    };
    if converted == 0 {
        Err(last_error().into())
    } else {
        Ok(descriptor)
    }
}

fn same_user_client(pipe: HANDLE) -> BridgeResult<bool> {
    let process_token = open_process_token()?;
    let process_user = token_user(process_token.0)?;
    if unsafe { ImpersonateNamedPipeClient(pipe) } == 0 {
        return Err(last_error().into());
    }
    let result = (|| {
        let thread_token = open_thread_token()?;
        let client_user = token_user(thread_token.0)?;
        let process_sid = unsafe { (*(process_user.as_ptr().cast::<TOKEN_USER>())).User.Sid };
        let client_sid = unsafe { (*(client_user.as_ptr().cast::<TOKEN_USER>())).User.Sid };
        Ok(unsafe { EqualSid(process_sid, client_sid) != 0 })
    })();
    unsafe {
        RevertToSelf();
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

fn open_process_token() -> BridgeResult<OwnedHandle> {
    let mut token = ptr::null_mut();
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &raw mut token) } == 0 {
        Err(last_error().into())
    } else {
        Ok(OwnedHandle(token))
    }
}

fn open_thread_token() -> BridgeResult<OwnedHandle> {
    let mut token = ptr::null_mut();
    if unsafe { OpenThreadToken(GetCurrentThread(), TOKEN_QUERY, 1, &raw mut token) } == 0 {
        Err(last_error().into())
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
        return Err(last_error().into());
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
        return Err(last_error().into());
    }
    Ok(bytes)
}

fn pipe_name(address: &EndpointAddress) -> BridgeResult<Vec<u16>> {
    let EndpointAddress::WindowsPipe { name } = address else {
        return Err(BridgeError::Unsupported("non-Windows endpoint"));
    };
    if !name.starts_with(r"\\.\pipe\spectrum-live-")
        || name.len() > 240
        || name[r"\\.\pipe\".len()..].contains(['\\', '/'])
    {
        return Err(BridgeError::Authentication(
            "named pipe is not a private local Spectrum endpoint".into(),
        ));
    }
    Ok(name.encode_utf16().chain(Some(0)).collect())
}

fn last_error() -> std::io::Error {
    std::io::Error::last_os_error()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn named_pipe_rejects_second_first_instance() {
        let address = EndpointAddress::WindowsPipe {
            name: format!(r"\\.\pipe\spectrum-live-{}", uuid::Uuid::new_v4()),
        };
        let _first = LocalListener::bind(&address).unwrap();
        assert!(LocalListener::bind(&address).is_err());
    }
}
