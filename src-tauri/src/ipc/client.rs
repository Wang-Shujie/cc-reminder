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
        use std::os::windows::io::RawHandle;
        use std::ptr::{null, null_mut};

        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::windows::named_pipe::NamedPipeClient;
        use tokio::time::timeout;
        use windows_sys::Win32::Foundation::{GENERIC_READ, GENERIC_WRITE, INVALID_HANDLE_VALUE};
        use windows_sys::Win32::Storage::FileSystem::{
            CreateFileW, FILE_FLAG_OVERLAPPED, OPEN_EXISTING,
        };
        use windows_sys::Win32::System::Pipes::WaitNamedPipeW;

        let Endpoint::Windows(name) = endpoint;
        let payload = serde_json::to_vec(request).map_err(|e| e.to_string())?;
        if payload.len() > MAX_HOOK_BYTES {
            return Err("frame too large".into());
        }
        let wide: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_io()
            .enable_time()
            .build()
            .map_err(|e| e.to_string())?;
        runtime.block_on(async {
            timeout(IPC_TOTAL_TIMEOUT, async {
                if unsafe { WaitNamedPipeW(wide.as_ptr(), IPC_CONNECT_TIMEOUT.as_millis() as u32) }
                    == 0
                {
                    return Err("connect timeout".into());
                }
                let handle = unsafe {
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
                if handle == INVALID_HANDLE_VALUE {
                    return Err("connect failed".into());
                }
                let mut stream = unsafe {
                    NamedPipeClient::from_raw_handle(handle as RawHandle)
                        .map_err(|error| error.to_string())?
                };
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
}
