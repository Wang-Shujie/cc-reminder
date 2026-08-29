use super::protocol::{
    IPC_CONNECT_TIMEOUT, IPC_TOTAL_TIMEOUT, IngressRequest, IngressResponse, MAX_HOOK_BYTES,
};
use super::server::Endpoint;

pub fn send_ingress(
    endpoint: &Endpoint,
    request: &IngressRequest,
) -> Result<IngressResponse, String> {
    #[cfg(unix)]
    {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::UnixStream;
        use tokio::time::timeout;

        let Endpoint::Unix(path) = endpoint;
        let payload = serde_json::to_vec(request).map_err(|e| e.to_string())?;
        if payload.len() > MAX_HOOK_BYTES {
            return Err("frame too large".into());
        }
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_io()
            .enable_time()
            .build()
            .map_err(|e| e.to_string())?;
        runtime.block_on(async {
            timeout(IPC_TOTAL_TIMEOUT, async {
                let mut stream = timeout(IPC_CONNECT_TIMEOUT, UnixStream::connect(path))
                    .await
                    .map_err(|_| "connect timeout".to_owned())?
                    .map_err(|e| e.to_string())?;
                stream
                    .write_all(&(payload.len() as u32).to_be_bytes())
                    .await
                    .map_err(|e| e.to_string())?;
                stream
                    .write_all(&payload)
                    .await
                    .map_err(|e| e.to_string())?;
                let mut length = [0; 4];
                stream
                    .read_exact(&mut length)
                    .await
                    .map_err(|e| e.to_string())?;
                let len = u32::from_be_bytes(length) as usize;
                if len > MAX_HOOK_BYTES {
                    return Err("response too large".into());
                }
                let mut bytes = vec![0; len];
                stream
                    .read_exact(&mut bytes)
                    .await
                    .map_err(|e| e.to_string())?;
                serde_json::from_slice(&bytes).map_err(|e| e.to_string())
            })
            .await
            .map_err(|_| "total timeout".to_owned())?
        })
    }
    #[cfg(windows)]
    {
        let Endpoint::Windows(name) = endpoint;
        let name = name.clone();
        let request = request.clone();
        run_with_deadline(IPC_TOTAL_TIMEOUT, "total timeout", move || {
            send_ingress_windows(&name, &request)
        })
    }
}

#[cfg(windows)]
fn send_ingress_windows(name: &str, request: &IngressRequest) -> Result<IngressResponse, String> {
    use std::os::windows::io::{IntoRawHandle, RawHandle};

    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::windows::named_pipe::NamedPipeClient;
    use tokio::time::timeout;

    let started = std::time::Instant::now();
    let payload = serde_json::to_vec(request).map_err(|error| error.to_string())?;
    if payload.len() > MAX_HOOK_BYTES {
        return Err("frame too large".into());
    }
    let wide: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
    let pipe = run_with_deadline(IPC_CONNECT_TIMEOUT, "connect timeout", move || {
        open_named_pipe(&wide)
    })?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .enable_time()
        .build()
        .map_err(|error| error.to_string())?;
    let remaining = IPC_TOTAL_TIMEOUT.saturating_sub(started.elapsed());
    if remaining.is_zero() {
        return Err("total timeout".into());
    }
    runtime.block_on(async move {
        let handle = pipe.into_raw_handle();
        // into_raw_handle 已把所有权移出 File;from_raw_handle 失败时句柄无主,
        // 必须在此关闭,否则每次失败泄漏一个管道句柄。
        let mut stream = unsafe {
            NamedPipeClient::from_raw_handle(handle as RawHandle).map_err(|error| {
                windows_sys::Win32::Foundation::CloseHandle(handle);
                error.to_string()
            })?
        };
        timeout(remaining, async {
            stream
                .write_all(&(payload.len() as u32).to_be_bytes())
                .await
                .map_err(|error| error.to_string())?;
            stream
                .write_all(&payload)
                .await
                .map_err(|error| error.to_string())?;
            let mut length = [0; 4];
            stream
                .read_exact(&mut length)
                .await
                .map_err(|error| error.to_string())?;
            let len = u32::from_be_bytes(length) as usize;
            if len > MAX_HOOK_BYTES {
                return Err("response too large".into());
            }
            let mut bytes = vec![0; len];
            stream
                .read_exact(&mut bytes)
                .await
                .map_err(|error| error.to_string())?;
            serde_json::from_slice(&bytes).map_err(|error| error.to_string())
        })
        .await
        .map_err(|_| "total timeout".to_owned())?
    })
}

#[cfg(windows)]
fn open_named_pipe(name: &[u16]) -> Result<std::fs::File, String> {
    use std::os::windows::io::{FromRawHandle, RawHandle};
    use std::ptr::{null, null_mut};

    use windows_sys::Win32::Foundation::{GENERIC_READ, GENERIC_WRITE, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, FILE_FLAG_OVERLAPPED, OPEN_EXISTING,
    };
    use windows_sys::Win32::System::Pipes::WaitNamedPipeW;

    if unsafe { WaitNamedPipeW(name.as_ptr(), IPC_CONNECT_TIMEOUT.as_millis() as u32) } == 0 {
        return Err("connect timeout".into());
    }
    let handle = unsafe {
        CreateFileW(
            name.as_ptr(),
            GENERIC_READ | GENERIC_WRITE,
            0,
            null(),
            OPEN_EXISTING,
            FILE_FLAG_OVERLAPPED,
            null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err("connect failed".into());
    }
    Ok(unsafe { std::fs::File::from_raw_handle(handle as RawHandle) })
}

#[cfg(windows)]
fn run_with_deadline<T, F>(
    timeout: std::time::Duration,
    timeout_error: &str,
    work: F,
) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, String> + Send + 'static,
{
    let deadline = std::time::Instant::now() + timeout;
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let _ = sender.send(work());
    });
    receiver
        .recv_timeout(deadline.saturating_duration_since(std::time::Instant::now()))
        .map_err(|_| timeout_error.to_owned())?
}

#[cfg(all(test, windows))]
mod windows_tests {
    use std::collections::BTreeMap;
    use std::io::Read;
    use std::os::windows::io::{AsRawHandle, FromRawHandle, RawHandle};
    use std::ptr::{null, null_mut};
    use std::time::{Duration, Instant};

    use windows_sys::Win32::Foundation::{
        ERROR_PIPE_CONNECTED, GENERIC_READ, GENERIC_WRITE, GetLastError, INVALID_HANDLE_VALUE,
    };
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, FILE_FLAG_OVERLAPPED, OPEN_EXISTING, PIPE_ACCESS_DUPLEX,
    };
    use windows_sys::Win32::System::Pipes::{
        ConnectNamedPipe, CreateNamedPipeW, PIPE_READMODE_BYTE, PIPE_TYPE_BYTE, PIPE_WAIT,
    };

    use crate::events::normalize::CapturedHookEvent;
    use crate::ipc::server::Endpoint;
    use crate::ipc::{
        IPC_CONNECT_TIMEOUT, IPC_PROTOCOL_VERSION, IPC_TOTAL_TIMEOUT, IngressRequest,
    };
    use crate::model::AgentKind;

    #[test]
    fn public_client_cannot_outlive_the_connect_deadline_when_all_instances_are_busy() {
        let name = format!(
            r"\\.\pipe\cc-reminder-connect-deadline-{}",
            uuid::Uuid::now_v7()
        );
        let wide: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
        let server_handle = unsafe {
            CreateNamedPipeW(
                wide.as_ptr(),
                PIPE_ACCESS_DUPLEX,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
                1,
                4096,
                4096,
                0,
                null(),
            )
        };
        assert_ne!(server_handle, INVALID_HANDLE_VALUE);
        let _server = unsafe { std::fs::File::from_raw_handle(server_handle as RawHandle) };
        let client_handle = unsafe {
            CreateFileW(
                wide.as_ptr(),
                GENERIC_READ | GENERIC_WRITE,
                0,
                null(),
                OPEN_EXISTING,
                FILE_FLAG_OVERLAPPED,
                null_mut(),
            )
        };
        assert_ne!(client_handle, INVALID_HANDLE_VALUE);
        let _client = unsafe { std::fs::File::from_raw_handle(client_handle as RawHandle) };

        let started = Instant::now();
        let result = super::send_ingress(&Endpoint::Windows(name), &request());

        assert_eq!(result.unwrap_err(), "connect timeout");
        assert!(started.elapsed() >= IPC_CONNECT_TIMEOUT);
        assert!(started.elapsed() < Duration::from_millis(150));
    }

    #[test]
    fn public_client_cannot_outlive_the_total_deadline_when_response_is_withheld() {
        let name = format!(
            r"\\.\pipe\cc-reminder-total-deadline-{}",
            uuid::Uuid::now_v7()
        );
        let wide: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
        let server_handle = unsafe {
            CreateNamedPipeW(
                wide.as_ptr(),
                PIPE_ACCESS_DUPLEX,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
                1,
                4096,
                4096,
                0,
                null(),
            )
        };
        assert_ne!(server_handle, INVALID_HANDLE_VALUE);
        let server = unsafe { std::fs::File::from_raw_handle(server_handle as RawHandle) };
        let handler = std::thread::spawn(move || {
            let connected = unsafe { ConnectNamedPipe(server.as_raw_handle(), null_mut()) } != 0
                || unsafe { GetLastError() } == ERROR_PIPE_CONNECTED;
            assert!(connected);
            let mut server = server;
            let mut length = [0; 4];
            server.read_exact(&mut length).unwrap();
            let mut body = vec![0; u32::from_be_bytes(length) as usize];
            server.read_exact(&mut body).unwrap();
            std::thread::sleep(Duration::from_millis(250));
        });
        let endpoint = Endpoint::Windows(name);

        let started = Instant::now();
        let result = super::send_ingress(&endpoint, &request());

        assert_eq!(result.unwrap_err(), "total timeout");
        assert!(started.elapsed() >= IPC_TOTAL_TIMEOUT);
        assert!(started.elapsed() < Duration::from_millis(200));
        handler.join().unwrap();
    }

    fn request() -> IngressRequest {
        IngressRequest {
            protocol_version: IPC_PROTOCOL_VERSION,
            helper_version: "0.1.0".into(),
            command_fingerprint: "deadline".into(),
            event: CapturedHookEvent {
                source: AgentKind::Codex,
                source_version: semver::Version::new(0, 145, 0),
                source_event: "Stop".into(),
                occurred_at: chrono::Utc::now(),
                cwd: None,
                session_id: None,
                turn_id: None,
                model: None,
                permission_mode: None,
                public_fields: BTreeMap::new(),
                sensitive_fields: BTreeMap::new(),
            },
        }
    }
}
