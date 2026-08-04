use super::protocol::{
    IPC_PROTOCOL_VERSION, IPC_TOTAL_TIMEOUT, IngressRequest, IngressResponse, MAX_HOOK_BYTES,
};
#[cfg(unix)]
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
            use std::os::unix::fs::PermissionsExt;
            use std::os::unix::net::UnixListener;
            let Endpoint::Unix(path) = &endpoint;
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700))
                    .map_err(|e| e.to_string())?;
            }
            let _ = std::fs::remove_file(path);
            let listener = UnixListener::bind(path).map_err(|e| e.to_string())?;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
                .map_err(|e| e.to_string())?;
            let (tx, rx) = mpsc::channel(64);
            std::thread::spawn(move || {
                for stream in listener.incoming().flatten() {
                    let tx = tx.clone();
                    std::thread::spawn(move || {
                        let mut stream = stream;
                        let _ = stream.set_read_timeout(Some(IPC_TOTAL_TIMEOUT));
                        let _ = stream.set_write_timeout(Some(IPC_TOTAL_TIMEOUT));
                        serve_stream(&mut stream, &tx);
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
            use std::os::windows::io::AsRawHandle;
            use std::ptr::null_mut;

            use windows_sys::Win32::Foundation::{ERROR_PIPE_CONNECTED, GetLastError};
            use windows_sys::Win32::System::Pipes::ConnectNamedPipe;

            let Endpoint::Windows(name) = &endpoint;
            let wide: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
            let first_pipe = create_named_pipe(&wide)?;
            let (tx, rx) = mpsc::channel(64);
            std::thread::spawn(move || {
                let mut first_pipe = Some(first_pipe);
                loop {
                    let pipe = match first_pipe.take() {
                        Some(pipe) => pipe,
                        None => match create_named_pipe(&wide) {
                            Ok(pipe) => pipe,
                            Err(_) => return,
                        },
                    };
                    let handle = pipe.as_raw_handle();
                    let connected = unsafe { ConnectNamedPipe(handle, null_mut()) } != 0
                        || unsafe { GetLastError() } == ERROR_PIPE_CONNECTED;
                    if !connected {
                        continue;
                    }
                    let tx = tx.clone();
                    std::thread::spawn(move || {
                        let mut pipe = pipe;
                        serve_stream(&mut pipe, &tx);
                    });
                }
            });
            Ok(Self {
                endpoint,
                receiver: rx,
            })
        }
    }
}

#[cfg(windows)]
fn create_named_pipe(name: &[u16]) -> Result<std::fs::File, String> {
    use std::os::windows::io::{FromRawHandle, RawHandle};

    use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
    use windows_sys::Win32::Storage::FileSystem::PIPE_ACCESS_DUPLEX;
    use windows_sys::Win32::System::Pipes::{
        CreateNamedPipeW, PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_WAIT,
    };

    use crate::security::permissions::{NamedPipeSecurity, verify_named_pipe_dacl};

    let security = NamedPipeSecurity::new().map_err(|error| error.code)?;
    let handle = unsafe {
        CreateNamedPipeW(
            name.as_ptr(),
            PIPE_ACCESS_DUPLEX,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            64,
            MAX_HOOK_BYTES as u32,
            MAX_HOOK_BYTES as u32,
            IPC_TOTAL_TIMEOUT.as_millis() as u32,
            security.attributes(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err("named pipe creation failed".into());
    }
    if !matches!(verify_named_pipe_dacl(handle), Ok(true)) {
        unsafe { windows_sys::Win32::Foundation::CloseHandle(handle) };
        return Err("named pipe DACL verification failed".into());
    }
    Ok(unsafe { std::fs::File::from_raw_handle(handle as RawHandle) })
}

fn serve_stream<S: std::io::Read + std::io::Write>(
    stream: &mut S,
    tx: &mpsc::Sender<(IngressRequest, mpsc::Sender<IngressResponse>)>,
) {
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
    let request = serde_json::from_slice::<IngressRequest>(&bytes)
        .map_err(|_| "invalid_request".to_string())
        .and_then(|request| {
            if request.protocol_version != IPC_PROTOCOL_VERSION {
                Err("protocol_version".into())
            } else {
                Ok(request)
            }
        });
    let response = if let Ok(request) = request {
        let (reply_tx, mut reply_rx) = mpsc::channel(1);
        if tx.try_send((request, reply_tx)).is_err() {
            IngressResponse::Rejected {
                error_code: "ipc_busy".into(),
            }
        } else {
            let deadline = std::time::Instant::now() + IPC_TOTAL_TIMEOUT;
            loop {
                match reply_rx.try_recv() {
                    Ok(reply) => break reply,
                    Err(mpsc::error::TryRecvError::Disconnected) => {
                        break IngressResponse::Rejected {
                            error_code: "ingress_unavailable".into(),
                        };
                    }
                    Err(mpsc::error::TryRecvError::Empty)
                        if std::time::Instant::now() < deadline =>
                    {
                        std::thread::sleep(std::time::Duration::from_millis(1));
                    }
                    Err(mpsc::error::TryRecvError::Empty) => {
                        break IngressResponse::Rejected {
                            error_code: "ingress_timeout".into(),
                        };
                    }
                }
            }
        }
    } else {
        IngressResponse::Rejected {
            error_code: "invalid_request".into(),
        }
    };
    if let Ok(body) = serde_json::to_vec(&response) {
        let _ = stream.write_all(&(body.len() as u32).to_be_bytes());
        let _ = stream.write_all(&body);
    }
}

#[cfg(all(test, unix))]
mod tests {
    use std::collections::BTreeMap;

    use super::serve_stream;
    use crate::events::normalize::{CapturedHookEvent, normalize_safe_ingress};
    use crate::hook_command::{insert_ingress, persist_ipc_request};
    use crate::ipc::{IPC_PROTOCOL_VERSION, IngressRequest, IngressResponse};
    use crate::model::AgentKind;
    use crate::paths::AppPaths;
    use crate::storage::db::Database;
    use chrono::Utc;
    use semver::Version;
    use tempfile::tempdir;

    #[test]
    fn accepted_request_is_in_ingress_before_success_response() {
        let root = tempdir().unwrap();
        let data_dir = root.path().join("com.ccreminder.app");
        let paths = paths(data_dir.clone());
        Database::open(&paths.database).unwrap();
        let (mut client_stream, mut server_stream) =
            std::os::unix::net::UnixStream::pair().unwrap();
        let (sender, mut receiver) = tokio::sync::mpsc::channel(1);
        std::thread::spawn(move || serve_stream(&mut server_stream, &sender));
        let client = std::thread::spawn(move || {
            use std::io::{Read, Write};

            let payload = serde_json::to_vec(&request()).unwrap();
            client_stream
                .write_all(&(payload.len() as u32).to_be_bytes())
                .unwrap();
            client_stream.write_all(&payload).unwrap();
            let mut header = [0; 4];
            client_stream.read_exact(&mut header).unwrap();
            let mut body = vec![0; u32::from_be_bytes(header) as usize];
            client_stream.read_exact(&mut body).unwrap();
            serde_json::from_slice::<IngressResponse>(&body).unwrap()
        });

        let (request, response) = receiver.blocking_recv().unwrap();
        let event_id = persist_ipc_request(&paths, request).unwrap();
        response
            .blocking_send(IngressResponse::Accepted { event_id })
            .unwrap();

        assert!(matches!(
            client.join().unwrap(),
            IngressResponse::Accepted { event_id: accepted } if accepted == event_id
        ));
        let connection = Database::open_ingress_writer(&paths.database).unwrap();
        let count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM ingress_events WHERE id = ?1",
                [event_id.to_string()],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn lost_accepted_then_helper_fallback_keeps_one_ingress_row() {
        use std::io::{Read, Write};

        let root = tempdir().unwrap();
        let paths = paths(root.path().join("com.ccreminder.app"));
        Database::open(&paths.database).unwrap();
        let original = request();
        let fallback_event = original.event.clone();
        let (mut client_stream, mut server_stream) =
            std::os::unix::net::UnixStream::pair().unwrap();
        let (sender, mut receiver) = tokio::sync::mpsc::channel(1);
        let server = std::thread::spawn(move || serve_stream(&mut server_stream, &sender));
        let client = std::thread::spawn(move || {
            let payload = serde_json::to_vec(&original).unwrap();
            client_stream
                .write_all(&(payload.len() as u32).to_be_bytes())
                .unwrap();
            client_stream.write_all(&payload).unwrap();
            let mut header = [0; 4];
            client_stream.read_exact(&mut header).unwrap();
            let mut body = vec![0; u32::from_be_bytes(header) as usize];
            client_stream.read_exact(&mut body).unwrap();
            serde_json::from_slice::<IngressResponse>(&body).unwrap()
        });

        let (received, lost_response) = receiver.blocking_recv().unwrap();
        let event_id = persist_ipc_request(&paths, received).unwrap();
        drop(lost_response);
        assert!(matches!(
            client.join().unwrap(),
            IngressResponse::Rejected { .. }
        ));
        server.join().unwrap();

        let key = crate::security::crypto::CorrelationKey::load_or_create(&paths.data_dir).unwrap();
        let fallback = normalize_safe_ingress(
            fallback_event,
            &[],
            crate::projects::PathPlatform::Unix,
            Some(key.expose_for_hmac()),
        );
        insert_ingress(&paths.database, &fallback).unwrap();

        let connection = Database::open_ingress_writer(&paths.database).unwrap();
        let count: i64 = connection
            .query_row("SELECT COUNT(*) FROM ingress_events", [], |row| row.get(0))
            .unwrap();
        assert_eq!(fallback.event_id, event_id);
        assert_eq!(count, 1);
    }

    fn request() -> IngressRequest {
        IngressRequest {
            protocol_version: IPC_PROTOCOL_VERSION,
            helper_version: "0.1.0".into(),
            command_fingerprint: "fingerprint".into(),
            event: CapturedHookEvent {
                source: AgentKind::Codex,
                source_version: Version::new(0, 145, 0),
                source_event: "Stop".into(),
                occurred_at: Utc::now(),
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

    fn paths(data_dir: std::path::PathBuf) -> AppPaths {
        AppPaths {
            database: data_dir.join("cc-reminder.sqlite3"),
            spool: data_dir.join("spool"),
            logs: data_dir.join("logs"),
            bin: data_dir.join("bin"),
            agent_versions: data_dir.join("agent-versions.json"),
            project_paths: data_dir.join("project-paths.json"),
            correlation_key: data_dir.join("correlation.key"),
            ipc: data_dir.join("ipc/hook.sock"),
            data_dir,
        }
    }
}

#[cfg(all(test, windows))]
mod windows_tests {
    use std::collections::BTreeMap;

    use super::{Endpoint, IpcServer};
    use crate::events::normalize::CapturedHookEvent;
    use crate::ipc::{IPC_PROTOCOL_VERSION, IngressRequest, IngressResponse};
    use crate::model::AgentKind;

    #[test]
    fn bind_reports_first_named_pipe_creation_failure() {
        assert!(IpcServer::bind(Endpoint::Windows("invalid-pipe-name".into())).is_err());
    }

    #[test]
    fn successful_bind_is_immediately_ready_for_a_named_pipe_round_trip() {
        let endpoint = Endpoint::Windows(format!(
            r"\\.\pipe\cc-reminder-readiness-{}",
            uuid::Uuid::now_v7()
        ));
        let mut server = IpcServer::bind(endpoint.clone()).unwrap();
        let handler = std::thread::spawn(move || {
            let (_, response) = server.receiver.blocking_recv().unwrap();
            response
                .blocking_send(IngressResponse::Accepted {
                    event_id: uuid::Uuid::nil(),
                })
                .unwrap();
        });

        assert!(matches!(
            crate::ipc::send_ingress(&endpoint, &request()),
            Ok(IngressResponse::Accepted { .. })
        ));
        handler.join().unwrap();
    }

    fn request() -> IngressRequest {
        IngressRequest {
            protocol_version: IPC_PROTOCOL_VERSION,
            helper_version: "0.1.0".into(),
            command_fingerprint: "readiness".into(),
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
