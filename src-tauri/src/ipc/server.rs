use super::protocol::{IPC_PROTOCOL_VERSION, IngressRequest, IngressResponse, MAX_HOOK_BYTES};
use std::path::PathBuf;
use tokio::sync::mpsc;

#[derive(Clone, Debug)]
pub enum Endpoint {
    #[cfg(unix)]
    Unix(PathBuf),
    #[cfg(windows)]
    Windows(String),
}

pub struct IpcServer {
    pub endpoint: Endpoint,
    pub receiver: mpsc::Receiver<(IngressRequest, mpsc::Sender<IngressResponse>)>,
}

impl IpcServer {
    pub fn bind(endpoint: Endpoint) -> Result<Self, String> {
        #[cfg(unix)]
        {
            use std::io::{Read, Write};
            use std::os::unix::fs::PermissionsExt;
            use std::os::unix::net::UnixListener;
            let Endpoint::Unix(path) = &endpoint;
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700)).ok();
            }
            let _ = std::fs::remove_file(path);
            let listener = UnixListener::bind(path).map_err(|e| e.to_string())?;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).ok();
            let (tx, rx) = mpsc::channel(64);
            std::thread::spawn(move || {
                for stream in listener.incoming().flatten() {
                    let tx = tx.clone();
                    std::thread::spawn(move || {
                        let mut stream = stream;
                        let mut header = [0; 4];
                        if stream.read_exact(&mut header).is_err() {
                            return;
                        }
                        let len = u32::from_be_bytes(header) as usize;
                        if len > MAX_HOOK_BYTES {
                            return;
                        }
                        let mut bytes = vec![0; len];
                        if stream.read_exact(&mut bytes).is_err() {
                            return;
                        }
                        let response = serde_json::from_slice::<IngressRequest>(&bytes)
                            .map_err(|_| "invalid_request".to_string())
                            .and_then(|request| {
                                if request.protocol_version != IPC_PROTOCOL_VERSION {
                                    Err("protocol_version".into())
                                } else {
                                    Ok(request)
                                }
                            });
                        if let Ok(request) = response {
                            let (reply_tx, mut reply_rx) = mpsc::channel(1);
                            let _ = tx.blocking_send((request, reply_tx));
                            if let Some(reply) = reply_rx.blocking_recv()
                                && let Ok(body) = serde_json::to_vec(&reply)
                            {
                                let _ = stream.write_all(&(body.len() as u32).to_be_bytes());
                                let _ = stream.write_all(&body);
                            }
                        }
                    });
                }
            });
            Ok(Self {
                endpoint,
                receiver: rx,
            })
        }
        #[cfg(windows)]
        {
            let _ = endpoint;
            Err("named pipe unavailable".into())
        }
    }
}
