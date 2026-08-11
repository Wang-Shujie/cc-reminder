#![cfg(feature = "test-support")]

//! Channel contract tests (Task 13).
//!
//! Verifies the hardened shared HTTP client, webhook host validation, DingTalk
//! signing fixed vector, markdown/text payload shape, text fallback on explicit
//! format rejection, response classification (timeouts / retries / auth / sig /
//! permission / format / malformed / oversized), redirect disabling, and the
//! content-limit boundaries for both platforms.
//!
//! All HTTP is served by a hand-rolled in-test TCP listener so we never touch
//! the real platforms and can assert exact request JSON.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use cc_reminder_lib::channels::{
    ChannelConfig, DingTalkSender, WeComSender, dingtalk_signature, validate_official_webhook,
};
use cc_reminder_lib::error::{ChannelSender, DeliveryErrorKind};
use cc_reminder_lib::model::{ChannelKind, NotificationDocument, Severity};

// -----------------------------------------------------------------------------
// Mock platform
// -----------------------------------------------------------------------------

/// A canned HTTP response the mock server will write back.
#[derive(Clone)]
struct CannedResponse {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl CannedResponse {
    fn json(status: u16, body: &Value) -> Self {
        let bytes = serde_json::to_vec(body).unwrap();
        Self {
            status,
            headers: vec![(
                "content-type".to_owned(),
                "application/json; charset=utf-8".to_owned(),
            )],
            body: bytes,
        }
    }

    fn status_only(status: u16) -> Self {
        Self {
            status,
            headers: vec![("content-length".to_owned(), "0".to_owned())],
            body: Vec::new(),
        }
    }
}

/// Behaviour the mock server runs for each accepted connection.
enum NextResponse {
    Static(CannedResponse),
    /// Redirect to the given location.
    Redirect(String),
}

struct MockInner {
    requests: Vec<(String, Vec<u8>)>,
    queue: Vec<NextResponse>,
}

/// Tiny in-test HTTP/1.1 server.
pub struct MockPlatform {
    address: String,
    inner: Arc<Mutex<MockInner>>,
    join: Option<tokio::task::JoinHandle<()>>,
}

impl MockPlatform {
    fn bind() -> (Self, tokio::task::JoinHandle<()>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap().to_string();
        listener.set_nonblocking(true).unwrap();
        let listener = TcpListener::from_std(listener).unwrap();

        let inner = Arc::new(Mutex::new(MockInner {
            requests: Vec::new(),
            queue: Vec::new(),
        }));
        let server_inner = inner.clone();
        let join = tokio::spawn(async move {
            loop {
                let (mut sock, _) = match listener.accept().await {
                    Ok(pair) => pair,
                    Err(_) => continue,
                };
                let inner = server_inner.clone();
                tokio::spawn(async move {
                    let _ = handle_connection(&mut sock, &inner).await;
                });
            }
        });
        let server = Self {
            address,
            inner,
            join: None,
        };
        (server, join)
    }

    fn new(queue: Vec<NextResponse>) -> Self {
        let (mut server, join) = Self::bind();
        server.join = Some(join);
        server.inner.lock().unwrap().queue = queue;
        server
    }

    fn endpoint(&self) -> String {
        format!("http://{}/", self.address)
    }

    fn captured_requests(&self) -> Vec<(String, Vec<u8>)> {
        self.inner.lock().unwrap().requests.clone()
    }

    fn first_body(&self) -> Value {
        let reqs = self.captured_requests();
        let body = reqs.first().map(|(_, body)| body.as_slice()).unwrap_or(&[]);
        serde_json::from_slice(body).unwrap_or(Value::Null)
    }

    fn nth_body(&self, idx: usize) -> Value {
        let reqs = self.captured_requests();
        let body = reqs
            .get(idx)
            .map(|(_, body)| body.as_slice())
            .unwrap_or(&[]);
        serde_json::from_slice(body).unwrap_or(Value::Null)
    }

    fn request_count(&self) -> usize {
        self.captured_requests().len()
    }
}

impl Drop for MockPlatform {
    fn drop(&mut self) {
        if let Some(join) = self.join.take() {
            join.abort();
        }
    }
}

async fn handle_connection(
    sock: &mut tokio::net::TcpStream,
    inner: &Arc<Mutex<MockInner>>,
) -> std::io::Result<()> {
    let _ = sock.set_nodelay(true);
    let mut buf = vec![0u8; 16 * 1024];
    // Read until we have the full headers (and as much body as fits in one read).
    let mut filled = 0usize;
    let header_end;
    loop {
        if filled == buf.len() {
            buf.resize(buf.len() * 2, 0);
        }
        let n = sock.read(&mut buf[filled..]).await?;
        if n == 0 {
            return Ok(());
        }
        filled += n;
        if let Some(idx) = find_double_crlf(&buf[..filled]) {
            header_end = idx + 4;
            break;
        }
    }
    let header_str = std::str::from_utf8(&buf[..header_end])
        .unwrap_or("")
        .to_owned();
    let mut request_line = "";
    let mut content_length = 0usize;
    for (i, line) in header_str.split("\r\n").enumerate() {
        if i == 0 {
            request_line = line;
        }
        let lower = line.to_ascii_lowercase();
        if let Some(rest) = lower.strip_prefix("content-length:") {
            content_length = rest.trim().parse().unwrap_or(0);
        }
    }
    let request_line = request_line.to_owned();
    // Read remaining body if the first read did not cover it.
    let body_start = header_end;
    let mut have = filled.saturating_sub(header_end);
    while have < content_length {
        if filled == buf.len() {
            buf.resize(buf.len() * 2, 0);
        }
        let n = sock.read(&mut buf[filled..]).await?;
        if n == 0 {
            break;
        }
        filled += n;
        have = filled.saturating_sub(body_start);
    }
    let body = buf[body_start..body_start + have.min(content_length)].to_vec();

    inner
        .lock()
        .unwrap()
        .requests
        .push((request_line.clone(), body.clone()));

    // Pop the next canned behaviour FIFO. If the queue is empty, default to 200 OK empty.
    let behaviour = { inner.lock().unwrap().queue.drain(..1).next() };
    let response = match behaviour {
        None => CannedResponse::status_only(200),
        Some(NextResponse::Static(canned)) => canned,
        Some(NextResponse::Redirect(location)) => CannedResponse {
            status: 302,
            headers: vec![("location".to_owned(), location)],
            body: Vec::new(),
        },
    };

    let mut out = Vec::new();
    let reason = reason_phrase(response.status);
    out.extend_from_slice(format!("HTTP/1.1 {} {}\r\n", response.status, reason).as_bytes());
    for (k, v) in &response.headers {
        out.extend_from_slice(format!("{}: {}\r\n", k, v).as_bytes());
    }
    out.extend_from_slice(format!("content-length: {}\r\n", response.body.len()).as_bytes());
    out.extend_from_slice(b"\r\n");
    out.extend_from_slice(&response.body);
    sock.write_all(&out).await?;
    sock.flush().await?;
    let _ = sock;
    Ok(())
}

fn find_double_crlf(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n")
}

fn reason_phrase(status: u16) -> &'static str {
    match status {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        408 => "Request Timeout",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        504 => "Gateway Timeout",
        _ => "Status",
    }
}

// -----------------------------------------------------------------------------
// Test helpers
// -----------------------------------------------------------------------------

fn document(body: &str) -> NotificationDocument {
    NotificationDocument {
        title: String::new(),
        severity: Severity::Info,
        facts: Vec::new(),
        body: body.to_owned(),
        footer: None,
    }
}

// -----------------------------------------------------------------------------
// Step 1: host and credential validation
// -----------------------------------------------------------------------------

#[test]
fn accepts_only_exact_official_https_hosts_and_paths() {
    assert!(
        validate_official_webhook(
            ChannelKind::DingTalk,
            "https://oapi.dingtalk.com/robot/send?access_token=fake",
        )
        .is_ok()
    );
    assert!(
        validate_official_webhook(
            ChannelKind::WeCom,
            "https://qyapi.weixin.qq.com/cgi-bin/webhook/send?key=fake",
        )
        .is_ok()
    );
    assert!(
        validate_official_webhook(
            ChannelKind::WeCom,
            "http://qyapi.weixin.qq.com/cgi-bin/webhook/send?key=fake",
        )
        .is_err()
    );
    assert!(
        validate_official_webhook(
            ChannelKind::DingTalk,
            "https://oapi.dingtalk.com.attacker.example/robot/send?access_token=fake",
        )
        .is_err()
    );
}

#[test]
fn rejects_subpaths_and_wrong_paths() {
    // extra path segment
    assert!(
        validate_official_webhook(
            ChannelKind::DingTalk,
            "https://oapi.dingtalk.com/robot/send/extra?access_token=fake",
        )
        .is_err()
    );
    // wrong path
    assert!(
        validate_official_webhook(
            ChannelKind::WeCom,
            "https://qyapi.weixin.qq.com/cgi-bin/notwebhook?key=fake",
        )
        .is_err()
    );
}

#[test]
fn rejects_wrong_host_kind() {
    assert!(
        validate_official_webhook(
            ChannelKind::WeCom,
            "https://oapi.dingtalk.com/robot/send?access_token=fake",
        )
        .is_err()
    );
    assert!(
        validate_official_webhook(
            ChannelKind::DingTalk,
            "https://qyapi.weixin.qq.com/cgi-bin/webhook/send?key=fake",
        )
        .is_err()
    );
}

#[test]
fn rejects_missing_token_or_key() {
    assert!(
        validate_official_webhook(
            ChannelKind::DingTalk,
            "https://oapi.dingtalk.com/robot/send",
        )
        .is_err()
    );
    assert!(
        validate_official_webhook(
            ChannelKind::WeCom,
            "https://qyapi.weixin.qq.com/cgi-bin/webhook/send",
        )
        .is_err()
    );
    assert!(
        validate_official_webhook(
            ChannelKind::DingTalk,
            "https://oapi.dingtalk.com/robot/send?access_token=",
        )
        .is_err()
    );
}

#[test]
fn rejects_extra_credential_query_names() {
    // DingTalk must only carry access_token.
    assert!(
        validate_official_webhook(
            ChannelKind::DingTalk,
            "https://oapi.dingtalk.com/robot/send?access_token=fake&token=other",
        )
        .is_err()
    );
    // WeCom must only carry key.
    assert!(
        validate_official_webhook(
            ChannelKind::WeCom,
            "https://qyapi.weixin.qq.com/cgi-bin/webhook/send?key=fake&access_token=x",
        )
        .is_err()
    );
}

#[test]
fn rejects_userinfo_fragment_port_and_ipliterals() {
    let bad = [
        "https://user:pass@oapi.dingtalk.com/robot/send?access_token=fake",
        "https://oapi.dingtalk.com/robot/send?access_token=fake#frag",
        "https://oapi.dingtalk.com:443/robot/send?access_token=fake",
        "https://oapi.dingtalk.com:8443/robot/send?access_token=fake",
        "https://127.0.0.1/robot/send?access_token=fake",
        "https://[::1]/robot/send?access_token=fake",
        "https://localhost/robot/send?access_token=fake",
    ];
    for url in bad {
        assert!(
            validate_official_webhook(ChannelKind::DingTalk, url).is_err(),
            "should reject {url}"
        );
    }
}

// -----------------------------------------------------------------------------
// Step 2a: DingTalk signing fixed vector
// -----------------------------------------------------------------------------

#[test]
fn dingtalk_signing_matches_fixed_vector() {
    assert_eq!(
        dingtalk_signature(1_609_459_200_000, "SECtest"),
        "p5mXVLdX%2FBTrc2KtuhTs6ZcGOXtsKU5g1oE3WtfH4hY%3D"
    );
}

// -----------------------------------------------------------------------------
// Step 2b: WeCom success + markdown payload shape
// -----------------------------------------------------------------------------

#[tokio::test]
async fn wecom_sends_markdown_and_maps_success() {
    let server = MockPlatform::new(vec![NextResponse::Static(CannedResponse::json(
        200,
        &json!({"errcode":0,"errmsg":"ok"}),
    ))]);
    let sender = WeComSender::for_contract_test(server.endpoint());
    let receipt = sender.send(&document("build complete")).await.unwrap();
    assert_eq!(receipt.http_status, 200);
    assert_eq!(receipt.platform_code.as_deref(), Some("0"));
    assert_eq!(
        server.first_body(),
        json!({"msgtype":"markdown","markdown":{"content":"build complete"}})
    );
}

#[tokio::test]
async fn dingtalk_sends_markdown_with_keyword_prefix_and_no_at_all() {
    let server = MockPlatform::new(vec![NextResponse::Static(CannedResponse::json(
        200,
        &json!({"errcode":0,"errmsg":"ok"}),
    ))]);
    let sender =
        DingTalkSender::for_contract_test(server.endpoint(), Some("CCReminder".to_owned()), None);
    let receipt = sender.send(&document("ship it")).await.unwrap();
    assert_eq!(receipt.platform_code.as_deref(), Some("0"));

    let body = server.first_body();
    assert_eq!(body["msgtype"], "markdown");
    let content = body["markdown"]["text"].as_str().unwrap();
    assert!(
        content.starts_with("CCReminder"),
        "keyword prefix must lead markdown, got: {content}"
    );
    // Never @all.
    assert!(body.get("at").is_none() || body["at"]["isAtAll"] != true);
}

#[tokio::test]
async fn dingtalk_signs_when_signing_secret_present() {
    let server = MockPlatform::new(vec![NextResponse::Static(CannedResponse::json(
        200,
        &json!({"errcode":0,"errmsg":"ok"}),
    ))]);
    let sender =
        DingTalkSender::for_contract_test(server.endpoint(), None, Some("SECtest".to_owned()));
    sender.send(&document("signed")).await.unwrap();

    let reqs = server.captured_requests();
    let path = &reqs[0].0;
    // timestamp + sign must be on the path (URL query), never in the body.
    assert!(path.contains("timestamp="), "path: {path}");
    assert!(path.contains("sign="), "path: {path}");
    let body = server.first_body();
    // sign/timestamp must NOT be echoed back inside the JSON body.
    assert!(body.get("timestamp").is_none());
    assert!(body.get("sign").is_none());
}

#[tokio::test]
async fn dingtalk_falls_back_to_text_on_explicit_format_rejection() {
    // First attempt: keyword/format rejection. Second: text accepted.
    let server = MockPlatform::new(vec![
        NextResponse::Static(CannedResponse::json(
            200,
            &json!({"errcode":310000,"errmsg":"keywords not in content"}),
        )),
        NextResponse::Static(CannedResponse::json(
            200,
            &json!({"errcode":0,"errmsg":"ok"}),
        )),
    ]);
    let sender =
        DingTalkSender::for_contract_test(server.endpoint(), Some("CCReminder".to_owned()), None);
    let receipt = sender.send(&document("ship")).await.unwrap();
    assert_eq!(receipt.platform_code.as_deref(), Some("0"));
    assert_eq!(server.request_count(), 2);
    // First markdown, second text.
    assert_eq!(server.first_body()["msgtype"], "markdown");
    let text_body = server.nth_body(1);
    assert_eq!(text_body["msgtype"], "text");
    let text = text_body["text"]["content"].as_str().unwrap();
    assert!(
        text.starts_with("CCReminder"),
        "text fallback must keep keyword prefix: {text}"
    );
}

#[tokio::test]
async fn dingtalk_does_not_fall_back_for_non_format_errors() {
    // errcode 300001 = token invalid -> Authentication, must NOT retry as text.
    let server = MockPlatform::new(vec![NextResponse::Static(CannedResponse::json(
        200,
        &json!({"errcode":300001,"errmsg":"token is invalid"}),
    ))]);
    let sender = DingTalkSender::for_contract_test(server.endpoint(), None, None);
    let err = sender.send(&document("x")).await.unwrap_err();
    assert_eq!(err.kind, DeliveryErrorKind::Authentication);
    assert_eq!(server.request_count(), 1);
}

// -----------------------------------------------------------------------------
// Step 2c: response classification
// -----------------------------------------------------------------------------

#[tokio::test]
async fn http_408_maps_to_http_status_error() {
    let server = MockPlatform::new(vec![NextResponse::Static(CannedResponse::status_only(408))]);
    let sender = WeComSender::for_contract_test(server.endpoint());
    let err = sender.send(&document("x")).await.unwrap_err();
    assert_eq!(err.kind, DeliveryErrorKind::HttpStatus);
    assert_eq!(err.http_status, Some(408));
}

#[tokio::test]
async fn http_429_parses_retry_after() {
    let server = MockPlatform::new(vec![NextResponse::Static(CannedResponse {
        status: 429,
        headers: vec![("retry-after".to_owned(), "30".to_owned())],
        body: b"{}".to_vec(),
    })]);
    let sender = WeComSender::for_contract_test(server.endpoint());
    let err = sender.send(&document("x")).await.unwrap_err();
    assert!(matches!(
        err.kind,
        DeliveryErrorKind::HttpStatus | DeliveryErrorKind::TemporaryPlatform
    ));
    assert_eq!(err.retry_after_seconds, Some(30));
}

#[tokio::test]
async fn http_5xx_maps_to_http_status() {
    for status in [500u16, 502, 503, 504] {
        let server = MockPlatform::new(vec![NextResponse::Static(CannedResponse::status_only(
            status,
        ))]);
        let sender = WeComSender::for_contract_test(server.endpoint());
        let err = sender.send(&document("x")).await.unwrap_err();
        assert_eq!(err.kind, DeliveryErrorKind::HttpStatus, "status {status}");
        assert_eq!(err.http_status, Some(status));
    }
}

#[tokio::test]
async fn wecom_temporary_platform_code_is_temporary() {
    // WeCom 45009 = api freq out of limit -> temporary.
    let server = MockPlatform::new(vec![NextResponse::Static(CannedResponse::json(
        200,
        &json!({"errcode":45009,"errmsg":"reach max freq limit"}),
    ))]);
    let sender = WeComSender::for_contract_test(server.endpoint());
    let err = sender.send(&document("x")).await.unwrap_err();
    assert_eq!(err.kind, DeliveryErrorKind::TemporaryPlatform);
    assert_eq!(err.platform_code.as_deref(), Some("45009"));
}

#[tokio::test]
async fn wecom_auth_and_permission_codes_are_permanent() {
    // 40014 invalid token, 48002 API forbidden.
    for (code, expected) in [
        (40014i64, DeliveryErrorKind::Authentication),
        (48002, DeliveryErrorKind::Permission),
    ] {
        let server = MockPlatform::new(vec![NextResponse::Static(CannedResponse::json(
            200,
            &json!({"errcode":code,"errmsg":"x"}),
        ))]);
        let sender = WeComSender::for_contract_test(server.endpoint());
        let err = sender.send(&document("x")).await.unwrap_err();
        assert_eq!(err.kind, expected, "code {code}");
    }
}

#[tokio::test]
async fn dingtalk_signature_rejection_maps_to_signature() {
    let server = MockPlatform::new(vec![NextResponse::Static(CannedResponse::json(
        200,
        &json!({"errcode":310000,"errmsg":"sign not match"}),
    ))]);
    let sender =
        DingTalkSender::for_contract_test(server.endpoint(), None, Some("SECtest".to_owned()));
    let err = sender.send(&document("ship")).await.unwrap_err();
    assert_eq!(err.kind, DeliveryErrorKind::Signature);
}

#[tokio::test]
async fn malformed_json_response_maps_to_format() {
    let server = MockPlatform::new(vec![NextResponse::Static(CannedResponse {
        status: 200,
        headers: vec![("content-type".to_owned(), "application/json".to_owned())],
        body: b"not json at all".to_vec(),
    })]);
    let sender = WeComSender::for_contract_test(server.endpoint());
    let err = sender.send(&document("x")).await.unwrap_err();
    assert_eq!(err.kind, DeliveryErrorKind::Format);
}

#[tokio::test]
async fn oversized_response_body_is_capped() {
    // 128 KiB body must be capped at 64 KiB and classified as Format/oversize.
    let big = vec![b'a'; 128 * 1024];
    let server = MockPlatform::new(vec![NextResponse::Static(CannedResponse {
        status: 200,
        headers: vec![("content-type".to_owned(), "application/json".to_owned())],
        body: big,
    })]);
    let sender = WeComSender::for_contract_test(server.endpoint());
    let err = sender.send(&document("x")).await.unwrap_err();
    assert_eq!(err.kind, DeliveryErrorKind::Format);
}

// -----------------------------------------------------------------------------
// Step 2d: timeouts + redirect disabled
// -----------------------------------------------------------------------------

#[tokio::test]
async fn connect_timeout_fires_on_unresponsive_listener() {
    // Bind a listener that accepts but never responds. The shared client must
    // give up within the request timeout (10s) -- we assert < 15s wall clock.
    // We can't easily force connect-timeout without a dropping blackhole; this
    // test exercises the request timeout path which is what the worker relies on.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    listener.set_nonblocking(true).unwrap();
    let listener = tokio::net::TcpListener::from_std(listener).unwrap();
    // Accept connections and HOLD them open without ever writing a response, so
    // the request hangs until the client's overall timeout fires.
    let join = tokio::spawn(async move {
        let mut held = Vec::new();
        while let Ok((stream, _)) = listener.accept().await {
            held.push(stream);
        }
        drop(held);
    });
    let endpoint = format!("http://{addr}/");
    let sender = WeComSender::for_contract_test(endpoint);
    let start = std::time::Instant::now();
    let err = sender.send(&document("x")).await.unwrap_err();
    let elapsed = start.elapsed();
    join.abort();
    assert!(
        matches!(
            err.kind,
            DeliveryErrorKind::Timeout | DeliveryErrorKind::Network
        ),
        "got {:?}",
        err.kind
    );
    assert!(
        elapsed.as_secs() < 15,
        "request timeout should fire well under 15s, took {elapsed:?}"
    );
}

#[tokio::test]
async fn redirects_are_disabled() {
    // Server returns 302 to an attacker host. The client must NOT follow; we
    // surface it as an HttpStatus error and only one request is made.
    let server = MockPlatform::new(vec![NextResponse::Redirect(
        "https://attacker.example/exfil".to_owned(),
    )]);
    let sender = WeComSender::for_contract_test(server.endpoint());
    let err = sender.send(&document("x")).await.unwrap_err();
    assert_eq!(err.kind, DeliveryErrorKind::HttpStatus);
    assert_eq!(err.http_status, Some(302));
    assert_eq!(
        server.request_count(),
        1,
        "client must not follow redirects"
    );
}

// -----------------------------------------------------------------------------
// Step 2e: content limits
// -----------------------------------------------------------------------------

#[tokio::test]
async fn wecom_markdown_at_byte_limit_is_accepted() {
    // WeCom markdown cap = 4096 UTF-8 bytes. Build a body whose rendered
    // markdown is exactly 4096 bytes.
    let server = MockPlatform::new(vec![NextResponse::Static(CannedResponse::json(
        200,
        &json!({"errcode":0,"errmsg":"ok"}),
    ))]);
    let sender = WeComSender::for_contract_test(server.endpoint());
    // Body string chosen so total content == 4096 bytes. We rely on the
    // renderer to truncate exactly at the cap, so feed something larger and
    // confirm the request body is exactly 4096 bytes long.
    let big = "a".repeat(5000);
    let receipt = sender.send(&document(&big)).await.unwrap();
    assert_eq!(receipt.platform_code.as_deref(), Some("0"));
    let body = server.first_body();
    let content = body["markdown"]["content"].as_str().unwrap();
    assert_eq!(content.len(), 4096);
}

#[tokio::test]
async fn wecom_markdown_one_over_byte_limit_truncates_cleanly() {
    let server = MockPlatform::new(vec![NextResponse::Static(CannedResponse::json(
        200,
        &json!({"errcode":0,"errmsg":"ok"}),
    ))]);
    let sender = WeComSender::for_contract_test(server.endpoint());
    // Multibyte content: we must not split a code point.
    let big = "中".repeat(2000); // 3 bytes each -> 6000 bytes
    sender.send(&document(&big)).await.unwrap();
    let body = server.first_body();
    let content = body["markdown"]["content"].as_str().unwrap();
    // Truncated to <= 4096 bytes and remains valid UTF-8 by construction.
    assert!(content.len() <= 4096);
    assert!(content.chars().all(|c| c == '中'));
}

#[tokio::test]
async fn dingtalk_markdown_truncates_at_char_limit() {
    let server = MockPlatform::new(vec![NextResponse::Static(CannedResponse::json(
        200,
        &json!({"errcode":0,"errmsg":"ok"}),
    ))]);
    let sender = DingTalkSender::for_contract_test(server.endpoint(), None, None);
    let big = "a".repeat(25_000);
    sender.send(&document(&big)).await.unwrap();
    let body = server.first_body();
    let content = body["markdown"]["text"].as_str().unwrap();
    assert!(content.chars().count() <= 20_000);
}

// -----------------------------------------------------------------------------
// Step 2f: ChannelConfig / health probe is a real send, not a GET
// -----------------------------------------------------------------------------

#[tokio::test]
async fn test_connection_uses_markdown_send_shape() {
    let server = MockPlatform::new(vec![NextResponse::Static(CannedResponse::json(
        200,
        &json!({"errcode":0,"errmsg":"ok"}),
    ))]);
    let sender = WeComSender::for_contract_test(server.endpoint());
    sender.test_connection().await.unwrap();
    // It must POST json, not GET.
    let reqs = server.captured_requests();
    assert!(reqs[0].0.starts_with("POST "), "got: {}", reqs[0].0);
    let body = server.first_body();
    assert_eq!(body["msgtype"], "markdown");
}

#[test]
fn channel_config_default_smoke() {
    let cfg = ChannelConfig::default();
    assert_eq!(cfg.connect_timeout(), Duration::from_secs(5));
    assert_eq!(cfg.request_timeout(), Duration::from_secs(10));
    assert_eq!(cfg.max_body_bytes(), 64 * 1024);
}
