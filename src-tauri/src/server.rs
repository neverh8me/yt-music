//! Local HTTP server the <audio> element plays from.
//! Serves downloaded files directly, otherwise proxies googlevideo with Range support,
//! so seeking works and stream URLs never have to reach the frontend.
//!
//! googlevideo rejects open-ended ranges (`bytes=0-`, which is what Chromium sends),
//! so the proxy always fetches upstream in bounded chunks and stitches them together.
use crate::state::Core;
use crate::vault;
use crate::yt::{Codec, StreamInfo};
use axum::Router;
use axum::body::Body;
use axum::extract::{Path, Request, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use std::sync::Arc;
use tower::ServiceExt;
use tower_http::services::ServeFile;

const CHUNK: u64 = 4 * 1024 * 1024;

pub async fn start(core: Arc<Core>) -> anyhow::Result<u16> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let port = listener.local_addr()?.port();
    let app = Router::new().route("/{token}/stream/{id}", get(stream)).with_state(core);
    tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, app).await {
            eprintln!("stream server stopped: {e}");
        }
    });
    Ok(port)
}

fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 32 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Parse a single `bytes=a-b` / `bytes=a-` / `bytes=-n` range against a known size.
fn parse_range(h: &str, total: u64) -> Option<(u64, u64)> {
    let spec = h.trim().strip_prefix("bytes=")?.split(',').next()?.trim();
    let (a, b) = spec.split_once('-')?;
    let last = total.checked_sub(1)?;
    let (start, end) = if a.is_empty() {
        let n: u64 = b.parse().ok()?;
        (total.saturating_sub(n), last)
    } else {
        let start: u64 = a.parse().ok()?;
        let end = if b.is_empty() { last } else { b.parse::<u64>().ok()?.min(last) };
        (start, end)
    };
    (start <= end).then_some((start, end))
}

fn clen_from_url(url: &str) -> Option<u64> {
    let q = url.split_once('?')?.1;
    q.split('&').find_map(|kv| kv.strip_prefix("clen=")).and_then(|v| v.parse().ok())
}

enum Upstream {
    /// URL is dead/forbidden — re-resolve and retry.
    Retry,
    Fail(StatusCode, String),
}

async fn total_size(core: &Core, info: &StreamInfo) -> Result<u64, Upstream> {
    if let Some(s) = info.size.or_else(|| clen_from_url(&info.url)) {
        return Ok(s);
    }
    let r = core
        .http
        .get(&info.url)
        .header(header::RANGE.as_str(), "bytes=0-0")
        .send()
        .await
        .map_err(|_| Upstream::Retry)?;
    if !r.status().is_success() {
        return Err(Upstream::Retry);
    }
    r.headers()
        .get(header::CONTENT_RANGE.as_str())
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.rsplit('/').next())
        .and_then(|v| v.parse().ok())
        .ok_or(Upstream::Fail(StatusCode::BAD_GATEWAY, "unknown stream size".into()))
}

async fn fetch_chunk(http: &reqwest::Client, url: &str, from: u64, to: u64) -> Result<reqwest::Response, Upstream> {
    let r = http
        .get(url)
        .header(header::RANGE.as_str(), format!("bytes={from}-{to}"))
        .send()
        .await
        .map_err(|_| Upstream::Retry)?;
    let s = r.status();
    if s == reqwest::StatusCode::PARTIAL_CONTENT || s == reqwest::StatusCode::OK {
        Ok(r)
    } else if s == reqwest::StatusCode::FORBIDDEN || s == reqwest::StatusCode::GONE || s.is_server_error() {
        Err(Upstream::Retry)
    } else {
        Err(Upstream::Fail(StatusCode::BAD_GATEWAY, format!("upstream {s}")))
    }
}

async fn proxy(core: &Core, info: &StreamInfo, range: Option<&str>) -> Result<Response, Upstream> {
    let total = total_size(core, info).await?;
    let (start, end) = match range {
        Some(r) => match parse_range(r, total) {
            Some(x) => x,
            None => {
                return Ok((
                    StatusCode::RANGE_NOT_SATISFIABLE,
                    [(header::CONTENT_RANGE, format!("bytes */{total}"))],
                )
                    .into_response());
            }
        },
        None => (0, total.saturating_sub(1)),
    };

    // Fetch the first chunk up front so a dead URL can still be retried.
    let first_end = end.min(start + CHUNK - 1);
    let first = fetch_chunk(&core.http, &info.url, start, first_end).await?;

    let http = core.http.clone();
    let url = info.url.clone();
    // state: (current response, next byte offset to request)
    let body = futures_util::stream::unfold((Some(first), first_end + 1), move |(mut cur, mut next)| {
        let http = http.clone();
        let url = url.clone();
        async move {
            loop {
                if let Some(resp) = cur.as_mut() {
                    match resp.chunk().await {
                        Ok(Some(bytes)) => return Some((Ok(bytes), (cur, next))),
                        Ok(None) => {} // this chunk is done; fetch the next one below
                        Err(e) => return Some((Err(std::io::Error::other(e)), (None, end + 1))),
                    }
                }
                if next > end {
                    return None;
                }
                let to = end.min(next + CHUNK - 1);
                match fetch_chunk(&http, &url, next, to).await {
                    Ok(r) => {
                        cur = Some(r);
                        next = to + 1;
                    }
                    Err(_) => {
                        let err = std::io::Error::other("upstream chunk failed");
                        return Some((Err(err), (None, end + 1)));
                    }
                }
            }
        }
    });

    let len = end - start + 1;
    let mut resp = Response::new(Body::from_stream(body));
    *resp.status_mut() = if range.is_some() { StatusCode::PARTIAL_CONTENT } else { StatusCode::OK };
    let h = resp.headers_mut();
    h.insert(header::CONTENT_LENGTH, HeaderValue::from(len));
    if range.is_some()
        && let Ok(v) = HeaderValue::from_str(&format!("bytes {start}-{end}/{total}"))
    {
        h.insert(header::CONTENT_RANGE, v);
    }
    h.insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    if let Ok(v) = HeaderValue::from_str(&info.mime) {
        h.insert(header::CONTENT_TYPE, v);
    }
    Ok(resp)
}

/// Decrypt a `.ytmc` file on the fly, honouring Range so seeking works offline.
async fn serve_vault(core: &Core, path: &str, range: Option<&str>) -> anyhow::Result<Response> {
    let mut file = tokio::fs::File::open(path).await?;
    let h = vault::read_header(&mut file).await?;
    let total = file.metadata().await?.len().saturating_sub(h.data_start);
    let (start, end) = match range {
        Some(r) => match parse_range(r, total) {
            Some(x) => x,
            None => {
                return Ok((
                    StatusCode::RANGE_NOT_SATISFIABLE,
                    [(header::CONTENT_RANGE, format!("bytes */{total}"))],
                )
                    .into_response());
            }
        },
        None => (0, total.saturating_sub(1)),
    };
    let len = end + 1 - start;
    let reader = vault::RangeReader::open(file, &h, &core.vault_key()?, start, len).await?;
    let body = futures_util::stream::unfold(reader, |mut r| async move {
        match r.next_chunk().await {
            Ok(Some(c)) => Some((Ok::<_, std::io::Error>(bytes_from(c)), r)),
            Ok(None) => None,
            Err(e) => Some((Err(std::io::Error::other(e)), r)),
        }
    });
    let mut resp = Response::new(Body::from_stream(body));
    *resp.status_mut() = if range.is_some() { StatusCode::PARTIAL_CONTENT } else { StatusCode::OK };
    let hd = resp.headers_mut();
    hd.insert(header::CONTENT_LENGTH, HeaderValue::from(len));
    if range.is_some() {
        hd.insert(header::CONTENT_RANGE, HeaderValue::from_str(&format!("bytes {start}-{end}/{total}"))?);
    }
    hd.insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    hd.insert(header::CONTENT_TYPE, HeaderValue::from_str(&h.mime)?);
    Ok(resp)
}

fn bytes_from(v: Vec<u8>) -> axum::body::Bytes {
    axum::body::Bytes::from(v)
}

/// Only answer requests addressed to the loopback host. A web page could otherwise use
/// DNS rebinding (evil.com → 127.0.0.1) to read responses from this server.
fn local_host(req: &Request, port: u16) -> bool {
    let Some(host) = req.headers().get(header::HOST).and_then(|h| h.to_str().ok()) else {
        return false;
    };
    host == format!("127.0.0.1:{port}") || host == format!("localhost:{port}")
}

/// Constant-time comparison so the token can't be guessed byte by byte from timings.
fn token_ok(given: &str, expected: &str) -> bool {
    given.len() == expected.len() && given.bytes().zip(expected.bytes()).fold(0u8, |acc, (a, b)| acc | (a ^ b)) == 0
}

/// Origins of the app's own webview. Only these get CORS, so the equalizer (Web Audio)
/// can process the stream; any other page still can't read the bytes.
const APP_ORIGINS: [&str; 3] = ["http://tauri.localhost", "https://tauri.localhost", "tauri://localhost"];

async fn stream(state: State<Arc<Core>>, path: Path<(String, String)>, req: Request) -> Response {
    let origin = req
        .headers()
        .get(header::ORIGIN)
        .and_then(|v| v.to_str().ok())
        .filter(|o| APP_ORIGINS.contains(o))
        .map(str::to_owned);
    let mut resp = stream_inner(state, path, req).await;
    if let Some(o) = origin
        && let Ok(v) = HeaderValue::from_str(&o)
    {
        let h = resp.headers_mut();
        h.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, v);
        h.insert(header::VARY, HeaderValue::from_static("Origin"));
    }
    resp
}

async fn stream_inner(State(core): State<Arc<Core>>, Path((token, id)): Path<(String, String)>, req: Request) -> Response {
    if !token_ok(&token, &core.stream_token) || !local_host(&req, core.port.load(std::sync::atomic::Ordering::Relaxed)) {
        return StatusCode::FORBIDDEN.into_response();
    }
    if !valid_id(&id) {
        return StatusCode::BAD_REQUEST.into_response();
    }

    // Offline copy wins.
    if let Some(path) = core.db.download_path(&id)
        && std::path::Path::new(&path).is_file()
    {
        if path.ends_with(&format!(".{}", vault::EXT)) {
            let range = req.headers().get(header::RANGE).and_then(|v| v.to_str().ok()).map(str::to_owned);
            return match serve_vault(&core, &path, range.as_deref()).await {
                Ok(r) => r,
                Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")).into_response(),
            };
        }
        // plain files from older versions
        return match ServeFile::new(&path).oneshot(req).await {
            Ok(r) => r.map(Body::new).into_response(),
            Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
        };
    }

    let range = req
        .headers()
        .get(header::RANGE)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);

    // attempt 0 may use a cached URL; attempt 1 resolves a fresh one.
    // Streaming uses AAC: it plays in every webview (WKWebView included).
    let mut last_err = String::from("stream unavailable");
    for _ in 0..2 {
        let info = match core.yt.stream(&id, Codec::Aac).await {
            Ok(i) => i,
            Err(e) => {
                last_err = format!("{e:#}");
                break;
            }
        };
        match proxy(&core, &info, range.as_deref()).await {
            Ok(r) => return r,
            Err(Upstream::Retry) => {
                last_err = "upstream refused the stream URL".into();
                core.yt.invalidate(&id);
            }
            Err(Upstream::Fail(code, msg)) => return (code, msg).into_response(),
        }
    }
    eprintln!("stream {id} failed: {last_err}");
    (StatusCode::BAD_GATEWAY, last_err).into_response()
}

#[cfg(test)]
mod tests {
    use super::parse_range;

    #[test]
    fn ranges() {
        assert_eq!(parse_range("bytes=0-", 100), Some((0, 99)));
        assert_eq!(parse_range("bytes=10-19", 100), Some((10, 19)));
        assert_eq!(parse_range("bytes=90-500", 100), Some((90, 99)));
        assert_eq!(parse_range("bytes=-10", 100), Some((90, 99)));
        assert_eq!(parse_range("bytes=100-", 100), None);
        assert_eq!(parse_range("items=0-1", 100), None);
    }
}

#[cfg(test)]
mod live_tests {
    use super::*;

    #[tokio::test]
    #[ignore]
    async fn live_proxy() {
        let core = crate::state::test_core();
        let id = core.yt.search("Океан Ельзи Обійми").await.unwrap().tracks[0].id.clone();
        let port = start(core.clone()).await.unwrap();
        core.port.store(port, std::sync::atomic::Ordering::Relaxed);
        let tok = &core.stream_token;
        // foreign Host header (DNS rebinding) must be refused
        let evil = reqwest::Client::new()
            .get(format!("http://127.0.0.1:{port}/{tok}/stream/{id}"))
            .header("Host", format!("evil.example:{port}"))
            .send().await.unwrap();
        assert_eq!(evil.status(), 403);
        // other local programs without the token must be refused too
        for bad in ["0000", "stream"] {
            let r = reqwest::get(format!("http://127.0.0.1:{port}/{bad}/stream/{id}")).await.unwrap();
            assert_eq!(r.status(), 403, "token {bad}");
        }
        let url = format!("http://127.0.0.1:{port}/{tok}/stream/{id}");
        let http = reqwest::Client::new();
        // what Chromium sends first
        let r = http.get(&url).header("Range", "bytes=0-").send().await.unwrap();
        let st = r.status();
        let cl: u64 = r.headers()["content-length"].to_str().unwrap().parse().unwrap();
        println!("open range: {st} len={cl} ct={:?} cr={:?}", r.headers().get("content-type"), r.headers().get("content-range"));
        let body = r.bytes().await.unwrap();
        println!("received {} bytes", body.len());
        assert_eq!(body.len() as u64, cl);
        // a seek in the middle
        let r = http.get(&url).header("Range", format!("bytes={}-", cl / 2)).send().await.unwrap();
        println!("seek: {} cr={:?}", r.status(), r.headers().get("content-range"));
        let n = r.bytes().await.unwrap().len() as u64;
        assert_eq!(n, cl - cl / 2);
    }
}
