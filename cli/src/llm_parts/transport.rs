// SPDX-License-Identifier: Apache-2.0
//! HTTP client construction, bounded stream waits, cancellation and DNS.
//! Transport does not interpret provider payloads or decide retries.

use std::env;
use std::net::{IpAddr, SocketAddr};
use std::sync::{mpsc, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use crate::execution_context::ExecutionContext;
use crate::i18n::{tr, Message};
use reqwest::blocking::{Client, ClientBuilder};

/// Construct a request target by editing only its URL path. Query pairs may
/// carry routing credentials and are neither rewritten nor included in errors.
pub(crate) fn normalized_model_endpoint(
    endpoint: &str,
    protocol: nola_domain::generation::ModelProtocol,
) -> Result<reqwest::Url, String> {
    use nola_domain::generation::ModelProtocol;
    let mut url = reqwest::Url::parse(endpoint.trim())
        .map_err(|_| "invalid model endpoint URL".to_string())?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return Err("model endpoint must be HTTP(S) without user-info or fragment".into());
    }
    let path = url.path().trim_end_matches('/');
    let path = match protocol {
        ModelProtocol::ChatCompletions if path.ends_with("/chat/completions") => path.to_owned(),
        ModelProtocol::ChatCompletions => format!("{path}/chat/completions"),
        ModelProtocol::Responses | ModelProtocol::SiwcResponses => {
            let path = path.strip_suffix("/chat/completions").unwrap_or(path);
            if path.ends_with("/responses") {
                path.to_owned()
            } else {
                format!("{path}/responses")
            }
        }
        ModelProtocol::Anthropic if path.ends_with("/messages") => path.to_owned(),
        ModelProtocol::Anthropic if path.ends_with("/v1") => format!("{path}/messages"),
        ModelProtocol::Anthropic => format!("{path}/v1/messages"),
    };
    url.set_path(&path);
    Ok(url)
}

pub(super) enum StreamLineRecv {
    Line(String),
    Error(String),
    Timeout,
    Disconnected,
    Cancelled,
}

pub(super) enum SendRequestResult {
    Response(reqwest::blocking::Response),
    Error(reqwest::Error),
    WorkerDisconnected,
    Cancelled,
}

pub(super) fn send_request_with_cancellation(
    request: reqwest::blocking::RequestBuilder,
    cancellation: &ExecutionContext,
) -> SendRequestResult {
    let measurement = nola_measurement::Span::physical_send();
    let (sender, receiver) = mpsc::sync_channel(1);
    thread::spawn(move || {
        let _ = sender.send(request.send());
    });
    await_request_response(receiver, cancellation, measurement)
}

fn await_request_response(
    receiver: mpsc::Receiver<Result<reqwest::blocking::Response, reqwest::Error>>,
    cancellation: &ExecutionContext,
    measurement: nola_measurement::Span,
) -> SendRequestResult {
    loop {
        if cancellation.is_requested() {
            measurement.finish(nola_measurement::Outcome::Cancelled);
            return SendRequestResult::Cancelled;
        }
        match receiver.recv_timeout(Duration::from_millis(50)) {
            Ok(Ok(response)) => {
                measurement.finish(if response.status().is_success() {
                    nola_measurement::Outcome::Succeeded
                } else { nola_measurement::Outcome::Rejected });
                return SendRequestResult::Response(response);
            }
            // reqwest attaches the complete request URL, potentially including
            // credential query pairs, to transport errors consumed by logs/UI.
            Ok(Err(error)) => {
                measurement.finish(nola_measurement::Outcome::Failed);
                return SendRequestResult::Error(error.without_url());
            }
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                measurement.finish(nola_measurement::Outcome::Failed);
                return SendRequestResult::WorkerDisconnected;
            }
        }
    }
}

#[cfg(test)]
mod request_worker_tests {
    use super::*;

    #[test]
    fn disconnected_worker_is_not_a_user_cancellation() {
        let (sender, receiver) = mpsc::sync_channel(1);
        drop(sender);
        let cancellation = ExecutionContext::default();
        let result = await_request_response(receiver, &cancellation,
            nola_measurement::Span::start(nola_measurement::Stage::HttpHeaders));
        assert!(matches!(result, SendRequestResult::WorkerDisconnected));
        assert!(!cancellation.is_requested());
    }

    #[test]
    fn explicit_cancellation_still_wins_when_worker_disconnects() {
        let (sender, receiver) = mpsc::sync_channel(1);
        drop(sender);
        let cancellation = ExecutionContext::default();
        cancellation.request();
        let result = await_request_response(receiver, &cancellation,
            nola_measurement::Span::start(nola_measurement::Stage::HttpHeaders));
        assert!(matches!(result, SendRequestResult::Cancelled));
    }
}

pub(super) fn response_retry_after_ms(response: &reqwest::blocking::Response) -> Option<u64> {
    if let Some(value) = response
        .headers()
        .get("retry-after-ms")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.trim().parse::<f64>().ok())
    {
        if value.is_finite() && value >= 0.0 {
            return Some(value.ceil().min(u64::MAX as f64) as u64);
        }
    }
    response
        .headers()
        .get("retry-after")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.trim().parse::<f64>().ok())
        .filter(|value| value.is_finite() && *value >= 0.0)
        .map(|seconds| (seconds * 1_000.0).ceil().min(u64::MAX as f64) as u64)
}

pub(super) fn recv_stream_line(
    line_rx: &mpsc::Receiver<Result<String, String>>,
    timeout: Duration,
    cancellation: &ExecutionContext,
) -> StreamLineRecv {
    let started = Instant::now();
    loop {
        if cancellation.is_requested() {
            return StreamLineRecv::Cancelled;
        }
        let elapsed = started.elapsed();
        if elapsed >= timeout {
            return StreamLineRecv::Timeout;
        }
        let wait = timeout
            .saturating_sub(elapsed)
            .min(Duration::from_millis(100));
        match line_rx.recv_timeout(wait) {
            Ok(Ok(line)) => return StreamLineRecv::Line(line),
            Ok(Err(err)) => return StreamLineRecv::Error(err),
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => return StreamLineRecv::Disconnected,
        }
    }
}

pub(super) fn llm_cancelled_message() -> String {
    "LLM request cancelled by user".to_string()
}

pub(super) fn classify_network_error_hint(error: &reqwest::Error) -> String {
    let message = format!("{error}");
    let lowered = message.to_ascii_lowercase();

    if lowered.contains("operation not permitted") || lowered.contains("os error 1") {
        return format!(
            "{} (execution environment blocked outbound network: operation not permitted)",
            message
        );
    }

    if lowered.contains("failed to lookup address information")
        || lowered.contains("failed to resolve")
        || lowered.contains("name or service not known")
        || lowered.contains("dns")
        || lowered.contains("nodename nor servname provided")
    {
        return format!(
            "{} (DNS resolution failed in runtime; check DNS or configure NOLA_LLM_DNS_FALLBACKS)",
            message
        );
    }

    if lowered.contains("timed out") || lowered.contains("timeout") {
        return format!("{} (request timeout; check endpoint reachability)", message);
    }

    if lowered.contains("connection refused")
        || lowered.contains("connection reset")
        || lowered.contains("connection aborted")
        || lowered.contains("connection closed")
    {
        return format!("{} (connection was rejected/closed by remote)", message);
    }

    if lowered.contains("network is unreachable")
        || lowered.contains("host unreachable")
        || lowered.contains("no route to host")
    {
        return format!("{} (runtime cannot reach endpoint host)", message);
    }

    message
}

pub(super) fn build_stream_client(
    endpoint_host: Option<&str>,
    resolved_addr: Option<SocketAddr>,
) -> Result<Client, String> {
    static DEFAULT_STREAM_CLIENT: OnceLock<Result<Client, String>> = OnceLock::new();
    if resolved_addr.is_none() {
        return DEFAULT_STREAM_CLIENT
            .get_or_init(|| {
                stream_client_builder()?
                    .build()
                    .map_err(|err| format!("{} {err}", tr(Message::FailedCreateHttpClientPrefix)))
            })
            .clone();
    }

    let mut builder = stream_client_builder()?;
    if let (Some(host), Some(addr)) = (endpoint_host, resolved_addr) {
        builder = builder.resolve(host, addr);
    }
    builder
        .build()
        .map_err(|err| format!("{} {err}", tr(Message::FailedCreateHttpClientPrefix)))
}

pub(super) fn build_native_stream_client() -> Result<Client, String> {
    static NATIVE_STREAM_CLIENT: OnceLock<Result<Client, String>> = OnceLock::new();
    NATIVE_STREAM_CLIENT.get_or_init(|| stream_client_builder()?
        .redirect(reqwest::redirect::Policy::none())
        .build().map_err(|error| format!("native model HTTP client unavailable: {error}")))
        .clone()
}

#[cfg(test)]
mod native_redirect_tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;

    #[test]
    fn native_client_surfaces_redirect_without_contacting_destination() {
        let source = TcpListener::bind("127.0.0.1:0").unwrap();
        let target = TcpListener::bind("127.0.0.1:0").unwrap();
        target.set_nonblocking(true).unwrap();
        let source_url = format!("http://{}/chat/completions", source.local_addr().unwrap());
        let target_url = format!("http://{}/unauthorized", target.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let (mut stream, _) = source.accept().unwrap();
            stream.set_read_timeout(Some(std::time::Duration::from_secs(5))).unwrap();
            let mut bytes = [0; 4096];
            assert!(stream.read(&mut bytes).unwrap() > 0);
            write!(stream, "HTTP/1.1 307 Temporary Redirect\r\nLocation: {target_url}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
        });
        let response = super::build_native_stream_client().unwrap().post(source_url)
            .body("{}").send().unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::TEMPORARY_REDIRECT);
        assert!(!response.status().is_success(), "provider dispatch rejects every non-success response");
        assert_eq!(target.accept().unwrap_err().kind(), std::io::ErrorKind::WouldBlock);
        server.join().unwrap();
    }
}

fn stream_client_builder() -> Result<ClientBuilder, String> {
    let mut builder = stream_client_builder_with_timeout(llm_http_timeout());
    // Managed environments can supply a private proxy CA. Keep certificate
    // verification enabled and honor the explicitly configured trust bundle.
    if let Some(path) = env::var_os("SSL_CERT_FILE").filter(|path| !path.is_empty()) {
        let pem = std::fs::read(path)
            .map_err(|_| "cannot read configured SSL_CERT_FILE".to_string())?;
        let certificates = reqwest::Certificate::from_pem_bundle(&pem)
            .map_err(|_| "invalid configured SSL_CERT_FILE".to_string())?;
        if certificates.is_empty() {
            return Err("configured SSL_CERT_FILE contains no certificates".into());
        }
        for certificate in certificates {
            builder = builder.add_root_certificate(certificate);
        }
    }
    Ok(builder)
}

fn stream_client_builder_with_timeout(timeout: Option<Duration>) -> ClientBuilder {
    // reqwest 0.11's BLOCKING timeout bounds the header wait and each body
    // read, not the lifetime of an actively streaming response. Keep that
    // protection; a RequestBuilder timeout would instead kill long streams.
    // Connection establishment has a separate bound (reqwest defaults to none).
    // None must be passed explicitly: the blocking builder otherwise keeps
    // reqwest's implicit 30-second timeout instead of disabling this layer.
    Client::builder()
        .connect_timeout(Duration::from_secs(30))
        .timeout(timeout)
}

#[cfg(test)]
mod stream_lifetime_tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};

    fn server(handler: impl FnOnce(TcpStream) + Send + 'static) -> (String, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/stream", listener.local_addr().unwrap());
        let worker = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
            stream.set_write_timeout(Some(Duration::from_secs(2))).unwrap();
            let mut request = Vec::new();
            let mut byte = [0];
            while !request.ends_with(b"\r\n\r\n") {
                stream.read_exact(&mut byte).unwrap();
                request.push(byte[0]);
                assert!(request.len() < 16_384);
            }
            handler(stream);
        });
        (url, worker)
    }

    #[test]
    fn active_stream_outlives_blocking_http_timeout() {
        let timeout = Duration::from_secs(1);
        let (release, released) = mpsc::channel();
        let (written, ready) = mpsc::channel();
        let (url, server) = server(move |mut stream| {
            write!(stream, "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n").unwrap();
            write!(stream, "1\r\nx\r\n").unwrap();
            stream.flush().unwrap();
            released.recv_timeout(Duration::from_secs(10)).unwrap();
            write!(stream, "9\r\nxxxxxxxxx\r\n0\r\n\r\n").unwrap();
            stream.flush().unwrap();
            written.send(()).unwrap();
        });
        let client = stream_client_builder_with_timeout(Some(timeout)).build().unwrap();
        let start = Instant::now();
        let mut response = client.get(url).send().unwrap();
        let mut first = [0];
        response.read_exact(&mut first).unwrap();
        assert_eq!(&first, b"x");
        // Cross the total request lifetime without an outstanding body read.
        // A total deadline would still expire here; a per-read idle deadline
        // must not. Release and acknowledge the remaining bytes explicitly,
        // rather than relying on an 80ms producer schedule in a 200ms window.
        thread::sleep(timeout * 2);
        release.send(()).unwrap();
        ready.recv_timeout(Duration::from_secs(10)).unwrap();
        let mut body = String::from("x");
        response.read_to_string(&mut body).unwrap();
        assert_eq!(body, "xxxxxxxxxx");
        assert!(start.elapsed() >= timeout * 2);
        server.join().unwrap();
    }

    #[test]
    fn disabled_http_timeout_allows_body_read_past_reqwest_default() {
        let (release, released) = mpsc::channel();
        let (url, server) = server(move |mut stream| {
            write!(stream, "HTTP/1.1 200 OK\r\nContent-Length: 1\r\nConnection: close\r\n\r\n").unwrap();
            stream.flush().unwrap();
            // Start the delay only once the client has received the headers.
            released.recv_timeout(Duration::from_secs(10)).unwrap();
            thread::sleep(Duration::from_secs(32));
            let _ = stream.write_all(b"x");
        });
        let client = stream_client_builder_with_timeout(None).build().unwrap();
        let mut response = client.get(url).send().unwrap();
        let start = Instant::now();
        release.send(()).unwrap();
        let mut body = String::new();
        let result = response.read_to_string(&mut body);
        server.join().unwrap();
        assert!(result.is_ok(), "disabled HTTP timeout inherited reqwest default: {result:?}");
        assert_eq!(body, "x");
        assert!(start.elapsed() >= Duration::from_secs(32));
    }

    #[test]
    fn http_timeout_configuration_has_explicit_disabled_semantics() {
        for value in ["none", "off", "false", "0", " NONE "] {
            assert_eq!(parse_llm_http_timeout(value), None, "{value}");
        }
        assert_eq!(parse_llm_http_timeout(" 42 "), Some(Duration::from_secs(42)));
        for value in ["", "-1", "invalid", "1.5"] {
            assert_eq!(parse_llm_http_timeout(value), Some(Duration::from_secs(180)));
        }
    }

    #[test]
    fn response_headers_cannot_wait_without_a_deadline() {
        let timeout = Duration::from_millis(80);
        let (url, server) = server(|_stream| thread::sleep(Duration::from_millis(300)));
        let client = stream_client_builder_with_timeout(Some(timeout)).build().unwrap();
        let error = client.get(url).send().unwrap_err();
        assert!(error.is_timeout(), "{error}");
        server.join().unwrap();
    }

    #[test]
    fn stalled_body_read_times_out_instead_of_becoming_clean_eof() {
        let timeout = Duration::from_millis(80);
        let (url, server) = server(|mut stream| {
            write!(stream, "HTTP/1.1 200 OK\r\nContent-Length: 10\r\nConnection: close\r\n\r\n").unwrap();
            stream.flush().unwrap();
            thread::sleep(Duration::from_millis(300));
        });
        let client = stream_client_builder_with_timeout(Some(timeout)).build().unwrap();
        let mut response = client.get(url).send().unwrap();
        let mut body = String::new();
        let error = response.read_to_string(&mut body).unwrap_err();
        assert!(error.to_string().contains("timed out"), "{error}");
        assert!(body.is_empty());
        server.join().unwrap();
    }
}

fn llm_http_timeout() -> Option<Duration> {
    let raw = env::var("NOLA_LLM_HTTP_TIMEOUT_SEC").unwrap_or_else(|_| "180".to_string());
    parse_llm_http_timeout(&raw)
}

fn parse_llm_http_timeout(raw: &str) -> Option<Duration> {
    let value = raw.trim();
    if value.eq_ignore_ascii_case("none")
        || value.eq_ignore_ascii_case("off")
        || value.eq_ignore_ascii_case("false")
        || value == "0"
    {
        return None;
    }
    let seconds = value
        .parse::<u64>()
        .ok()
        .filter(|seconds| *seconds > 0)
        .unwrap_or(180);
    Some(Duration::from_secs(seconds))
}

pub(super) fn stream_idle_timeout() -> Duration {
    let seconds = env::var("NOLA_LLM_STREAM_IDLE_TIMEOUT_SEC")
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .filter(|value| (1..=3600).contains(value))
        .unwrap_or(120);
    Duration::from_secs(seconds)
}

pub(super) fn stream_first_event_timeout(stream_idle_timeout: Duration) -> Duration {
    let seconds = env::var("NOLA_LLM_STREAM_FIRST_EVENT_TIMEOUT_SEC")
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .filter(|value| (1..=3600).contains(value));
    seconds
        .map(Duration::from_secs)
        .unwrap_or_else(|| stream_idle_timeout.min(Duration::from_secs(60)))
}

pub(super) fn stream_tail_idle_timeout() -> Duration {
    let seconds = env::var("NOLA_LLM_STREAM_TAIL_IDLE_TIMEOUT_SEC")
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .filter(|value| (1..=3600).contains(value))
        .unwrap_or(30);
    Duration::from_secs(seconds)
}

pub(super) fn resolve_dns_fallbacks_for_host(host: &str, endpoint_port: u16) -> Vec<SocketAddr> {
    let fallback_map = env::var("NOLA_LLM_DNS_FALLBACKS")
        .or_else(|_| env::var("NOLA_CLI_DNS_FALLBACKS"))
        .unwrap_or_default();
    let mut out = Vec::new();
    let target_host = host.trim().to_ascii_lowercase();
    if fallback_map.trim().is_empty() || target_host.is_empty() {
        return out;
    }

    for entry in fallback_map.split([',', ';']) {
        let Some((raw_host, raw_addr)) = entry.split_once('=') else {
            continue;
        };
        if raw_host.trim().to_ascii_lowercase() != target_host {
            continue;
        }
        let addr_text = raw_addr.trim();
        if addr_text.is_empty() {
            continue;
        }
        if let Ok(addr) = addr_text.parse::<SocketAddr>() {
            if addr.port() == 0 {
                out.push(SocketAddr::new(addr.ip(), endpoint_port));
            } else {
                out.push(addr);
            }
            continue;
        }
        if let Ok(ip) = addr_text.parse::<IpAddr>() {
            out.push(SocketAddr::new(ip, endpoint_port));
            continue;
        }
    }

    out
}
