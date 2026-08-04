use super::protocol::{IngressRequest, IngressResponse, MAX_HOOK_BYTES};
use super::server::Endpoint;
use std::io::{Read, Write};
use std::time::Duration;

pub fn send_ingress(
    endpoint: &Endpoint,
    request: &IngressRequest,
) -> Result<IngressResponse, String> {
    #[cfg(unix)]
    {
        use std::os::unix::net::UnixStream;
        let Endpoint::Unix(path) = endpoint;
        let mut stream = UnixStream::connect(path).map_err(|e| e.to_string())?;
        stream
            .set_read_timeout(Some(Duration::from_millis(75)))
            .ok();
        let payload = serde_json::to_vec(request).map_err(|e| e.to_string())?;
        if payload.len() > MAX_HOOK_BYTES {
            return Err("frame too large".into());
        }
        stream
            .write_all(&(payload.len() as u32).to_be_bytes())
            .map_err(|e| e.to_string())?;
        stream.write_all(&payload).map_err(|e| e.to_string())?;
        let mut length = [0; 4];
        stream.read_exact(&mut length).map_err(|e| e.to_string())?;
        let len = u32::from_be_bytes(length) as usize;
        if len > MAX_HOOK_BYTES {
            return Err("response too large".into());
        }
        let mut bytes = vec![0; len];
        stream.read_exact(&mut bytes).map_err(|e| e.to_string())?;
        serde_json::from_slice(&bytes).map_err(|e| e.to_string())
    }
    #[cfg(windows)]
    {
        let _ = (endpoint, request);
        Err("named pipe unavailable".into())
    }
}
