use std::{
    ffi::c_void,
    io::{Read, Write},
    ptr,
    sync::Mutex,
    time::Duration,
};

use windows_sys::Win32::{
    Foundation::{
        CloseHandle, DUPLICATE_SAME_ACCESS, DuplicateHandle, ERROR_IO_PENDING,
        ERROR_PIPE_CONNECTED, GENERIC_READ, GENERIC_WRITE, GetLastError, HANDLE,
        INVALID_HANDLE_VALUE, WAIT_OBJECT_0, WAIT_TIMEOUT,
    },
    Security::{
        EqualSid, GetTokenInformation, RevertToSelf, SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER,
        TokenUser,
    },
    Storage::FileSystem::{
        CreateFileW, FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_FLAG_OVERLAPPED, OPEN_EXISTING,
        PIPE_ACCESS_DUPLEX, ReadFile, WriteFile,
    },
    System::{
        IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED},
        Pipes::{
            ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, ImpersonateNamedPipeClient,
            PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_WAIT,
        },
        Threading::{
            CreateEventW, GetCurrentProcess, GetCurrentThread, INFINITE, OpenProcessToken,
            OpenThreadToken, WaitForSingleObject,
        },
    },
};

use crate::{
    BridgeError, BridgeResult, EndpointAddress,
    windows_security::{OwnedSecurityDescriptor, current_user_only_descriptor},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PeerIdentity {
    pub same_user: bool,
}

pub struct LocalListener {
    name: Vec<u16>,
    pending: Mutex<HANDLE>,
    descriptor: OwnedSecurityDescriptor,
}

unsafe impl Send for LocalListener {}
unsafe impl Sync for LocalListener {}

impl LocalListener {
    pub fn bind(address: &EndpointAddress) -> BridgeResult<Self> {
        let name = pipe_name(address)?;
        let descriptor = current_user_only_descriptor()?;
        let handle = create_pipe(&name, descriptor.as_ptr(), true);
        if handle == INVALID_HANDLE_VALUE {
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
        connect_overlapped(*pending)?;
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
        let replacement = create_pipe(&self.name, self.descriptor.as_ptr(), false);
        if replacement == INVALID_HANDLE_VALUE {
            unsafe {
                DisconnectNamedPipe(accepted);
                CloseHandle(accepted);
            }
            return Err(last_error().into());
        }
        *pending = replacement;
        Ok((LocalStream::new(accepted, true), PeerIdentity { same_user }))
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
    }
}

pub struct LocalStream {
    handle: HANDLE,
    read_timeout: Mutex<Option<Duration>>,
    write_timeout: Mutex<Option<Duration>>,
    disconnect_on_drop: bool,
}

unsafe impl Send for LocalStream {}

impl LocalStream {
    fn new(handle: HANDLE, disconnect_on_drop: bool) -> Self {
        Self {
            handle,
            read_timeout: Mutex::new(None),
            write_timeout: Mutex::new(None),
            disconnect_on_drop,
        }
    }

    pub fn connect(address: &EndpointAddress) -> BridgeResult<Self> {
        let name = pipe_name(address)?;
        let handle = unsafe {
            CreateFileW(
                name.as_ptr(),
                GENERIC_READ | GENERIC_WRITE,
                0,
                ptr::null(),
                OPEN_EXISTING,
                FILE_FLAG_OVERLAPPED,
                ptr::null_mut(),
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            return Err(last_error().into());
        }
        Ok(Self::new(handle, false))
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
        let read_timeout = *self.read_timeout.lock().map_err(|_| BridgeError::Closed)?;
        let write_timeout = *self.write_timeout.lock().map_err(|_| BridgeError::Closed)?;
        Ok(Self {
            handle: duplicate,
            read_timeout: Mutex::new(read_timeout),
            write_timeout: Mutex::new(write_timeout),
            disconnect_on_drop: false,
        })
    }

    pub fn set_read_timeout(&self, timeout: Option<Duration>) -> BridgeResult<()> {
        *self.read_timeout.lock().map_err(|_| BridgeError::Closed)? = timeout;
        Ok(())
    }

    pub fn set_write_timeout(&self, timeout: Option<Duration>) -> BridgeResult<()> {
        *self.write_timeout.lock().map_err(|_| BridgeError::Closed)? = timeout;
        Ok(())
    }

    pub fn shutdown(&self) -> BridgeResult<()> {
        unsafe {
            CancelIoEx(self.handle, ptr::null());
            if self.disconnect_on_drop {
                DisconnectNamedPipe(self.handle);
            }
        }
        Ok(())
    }
}

impl Read for LocalStream {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        let timeout = *self
            .read_timeout
            .lock()
            .map_err(|_| std::io::Error::other("read timeout mutex poisoned"))?;
        let length = u32::try_from(buffer.len()).unwrap_or(u32::MAX);
        overlapped_io(self.handle, timeout, |overlapped, transferred| unsafe {
            ReadFile(
                self.handle,
                buffer.as_mut_ptr(),
                length,
                transferred,
                overlapped,
            )
        })
    }
}

impl Write for LocalStream {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        let timeout = *self
            .write_timeout
            .lock()
            .map_err(|_| std::io::Error::other("write timeout mutex poisoned"))?;
        let length = u32::try_from(buffer.len()).unwrap_or(u32::MAX);
        overlapped_io(self.handle, timeout, |overlapped, transferred| unsafe {
            WriteFile(
                self.handle,
                buffer.as_ptr(),
                length,
                transferred,
                overlapped,
            )
        })
    }

    fn flush(&mut self) -> std::io::Result<()> {
        // Each overlapped WriteFile completion has already transferred the
        // frame into the pipe. FlushFileBuffers can wait indefinitely for the
        // peer to consume all bytes and has no timeout; never use it on the
        // connection hot path.
        Ok(())
    }
}

impl Drop for LocalStream {
    fn drop(&mut self) {
        unsafe {
            if self.disconnect_on_drop {
                DisconnectNamedPipe(self.handle);
            }
            CloseHandle(self.handle);
        }
    }
}

fn connect_overlapped(handle: HANDLE) -> BridgeResult<()> {
    let event = OwnedHandle::event()?;
    let mut overlapped: OVERLAPPED = unsafe { std::mem::zeroed() };
    overlapped.hEvent = event.0;
    let connected = unsafe { ConnectNamedPipe(handle, &raw mut overlapped) };
    if connected != 0 {
        return Ok(());
    }
    match unsafe { GetLastError() } {
        ERROR_PIPE_CONNECTED => Ok(()),
        ERROR_IO_PENDING => {
            if unsafe { WaitForSingleObject(event.0, INFINITE) } != WAIT_OBJECT_0 {
                let error = last_error();
                cancel_and_drain(handle, &overlapped);
                return Err(error.into());
            }
            let mut transferred = 0;
            if unsafe { GetOverlappedResult(handle, &overlapped, &raw mut transferred, 0) } == 0 {
                Err(last_error().into())
            } else {
                Ok(())
            }
        }
        _ => Err(last_error().into()),
    }
}

fn overlapped_io(
    handle: HANDLE,
    timeout: Option<Duration>,
    operation: impl FnOnce(*mut OVERLAPPED, *mut u32) -> i32,
) -> std::io::Result<usize> {
    let event = OwnedHandle::event()?;
    let mut overlapped: OVERLAPPED = unsafe { std::mem::zeroed() };
    overlapped.hEvent = event.0;
    let mut transferred = 0_u32;
    let started = operation(&raw mut overlapped, &raw mut transferred);
    if started == 0 {
        let error = unsafe { GetLastError() };
        if error != ERROR_IO_PENDING {
            return Err(std::io::Error::from_raw_os_error(error as i32));
        }
        let wait = timeout.map_or(INFINITE, duration_millis);
        match unsafe { WaitForSingleObject(event.0, wait) } {
            WAIT_OBJECT_0 => {}
            WAIT_TIMEOUT => {
                cancel_and_drain(handle, &overlapped);
                return Err(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "named-pipe operation timed out",
                ));
            }
            _ => {
                let error = last_error();
                cancel_and_drain(handle, &overlapped);
                return Err(error);
            }
        }
        if unsafe { GetOverlappedResult(handle, &overlapped, &raw mut transferred, 0) } == 0 {
            return Err(last_error());
        }
    }
    Ok(transferred as usize)
}

fn cancel_and_drain(handle: HANDLE, overlapped: &OVERLAPPED) {
    let mut transferred = 0;
    unsafe {
        CancelIoEx(handle, overlapped);
        // The OVERLAPPED lives on the caller's stack, so never return while
        // the kernel may still reference it. ERROR_OPERATION_ABORTED is the
        // expected result after cancellation and still proves completion.
        GetOverlappedResult(handle, overlapped, &raw mut transferred, 1);
    }
}

fn duration_millis(duration: Duration) -> u32 {
    u32::try_from(duration.as_millis().max(1)).unwrap_or(u32::MAX - 1)
}

fn create_pipe(
    name: &[u16],
    descriptor: windows_sys::Win32::Security::PSECURITY_DESCRIPTOR,
    first: bool,
) -> HANDLE {
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
            PIPE_ACCESS_DUPLEX | FILE_FLAG_OVERLAPPED | first_flag,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            crate::MAX_AUTHENTICATED_CONNECTIONS as u32,
            64 * 1024,
            64 * 1024,
            0,
            &raw mut attributes,
        )
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

impl OwnedHandle {
    fn event() -> std::io::Result<Self> {
        let handle = unsafe { CreateEventW(ptr::null(), 1, 0, ptr::null()) };
        if handle.is_null() {
            Err(last_error())
        } else {
            Ok(Self(handle))
        }
    }
}

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
    use std::{io::Write as _, thread, time::Instant};

    use super::*;

    fn address() -> EndpointAddress {
        EndpointAddress::WindowsPipe {
            name: format!(r"\\.\pipe\spectrum-live-{}", uuid::Uuid::new_v4()),
        }
    }

    #[test]
    fn named_pipe_rejects_second_first_instance() {
        let address = address();
        let _first = LocalListener::bind(&address).unwrap();
        assert!(LocalListener::bind(&address).is_err());
    }

    #[test]
    fn current_user_peer_and_real_read_timeout_are_enforced() {
        let address = address();
        let listener =
            LocalListener::bind(&address).expect("bind current-user-only first pipe instance");
        let client = thread::spawn({
            let address = address.clone();
            move || LocalStream::connect(&address).expect("connect same-user pipe client")
        });
        let (mut server, identity) = listener
            .accept()
            .expect("accept and verify same-user pipe client");
        let _client = client.join().unwrap();
        assert!(identity.same_user);
        server
            .set_read_timeout(Some(Duration::from_millis(50)))
            .unwrap();
        let started = Instant::now();
        let error = server.read(&mut [0_u8; 1]).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::TimedOut);
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn real_write_timeout_cancels_pending_overlapped_io() {
        let address = address();
        let listener =
            LocalListener::bind(&address).expect("bind current-user-only first pipe instance");
        let client = thread::spawn({
            let address = address.clone();
            move || LocalStream::connect(&address).expect("connect non-reading pipe client")
        });
        let (mut server, _) = listener
            .accept()
            .expect("accept non-reading same-user pipe client");
        let _client = client.join().unwrap();
        server
            .set_write_timeout(Some(Duration::from_millis(50)))
            .unwrap();
        let started = Instant::now();
        let payload = vec![0_u8; 64 * 1024];
        let error = (0..16)
            .find_map(|_| server.write_all(&payload).err())
            .expect("named pipe writes never reached the bounded timeout");
        assert_eq!(error.kind(), std::io::ErrorKind::TimedOut);
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn nonlocal_pipe_names_are_rejected_before_open() {
        let address = EndpointAddress::WindowsPipe {
            name: format!(r"\\localhost\pipe\spectrum-live-{}", uuid::Uuid::new_v4()),
        };
        assert!(LocalStream::connect(&address).is_err());
    }
}
