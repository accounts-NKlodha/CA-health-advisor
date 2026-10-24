//! The `/v1` LAN API (V1-5 contract 8.1), served only over the pinned TLS listener.
//!
//! Common rules: the user id always comes from the token, never from a path, query,
//! body or header; errors are one small enum with no free text; request bodies are never
//! logged. A pending device may call only `GET /v1/state` and `POST /v1/unpair`: every
//! other authenticated endpoint refuses it with `device_unconfirmed` **before** any other
//! processing (contract 4.5).
//!
//! Every request from a confirmed device may carry its stored receipt in `HY-Receipt`
//! ([`Server::check_echo`]): an older receipt than before answers `device_regressed`.
//! `POST /v1/receipt` counts as evidence only for the device whose token presents it; a
//! receipt sent without a usable token is refused, and is only checked for a history
//! beyond the server's (contract 9). The update manifest is V1-6 and answers
//! `protocol_unsupported` after the gate.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Instant;

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::extract::{Extension, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use hy_core::protocol::unb64_exact;
use hy_core::protocol::wire::{
    ControlRequest, ErrorBody, PairRequest, RECEIPT_HEADER, ReceiptRequest, UploadRequest,
    WireError,
};

use crate::logging::{self, V};
use crate::server::{AuthedDevice, DeviceState, Refusal, Server};

/// The connecting peer's address, attached to each connection by the accept loop.
#[derive(Debug, Clone, Copy)]
pub struct Peer(pub SocketAddr);

/// Request body limit for every endpoint (contract 5).
pub const BODY_LIMIT: usize = 256 * 1024;
/// Request body limit for `/v1/pair`.
pub const PAIR_BODY_LIMIT: usize = 4 * 1024;

/// The HTTP status for each wire error.
#[must_use]
pub fn status_of(e: WireError) -> StatusCode {
    match e {
        WireError::UnknownToken | WireError::DeviceRevoked => StatusCode::UNAUTHORIZED,
        WireError::DeviceUnconfirmed | WireError::SlipInvalid => StatusCode::FORBIDDEN,
        WireError::RateLimited => StatusCode::TOO_MANY_REQUESTS,
        WireError::OriginConflict | WireError::StaleEnable | WireError::DeviceRegressed => {
            StatusCode::CONFLICT
        }
        WireError::RecoveryHold => StatusCode::LOCKED,
        WireError::PayloadTooLarge => StatusCode::PAYLOAD_TOO_LARGE,
        WireError::ProtocolUnsupported => StatusCode::NOT_FOUND,
        WireError::CannotRecord => StatusCode::SERVICE_UNAVAILABLE,
        WireError::AcceptedPendingRecovery => StatusCode::ACCEPTED,
        _ => StatusCode::BAD_REQUEST,
    }
}

fn error_response(e: WireError) -> Response {
    let mut r = (status_of(e), axum::Json(ErrorBody { error: e })).into_response();
    r.headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    r
}

fn json_response<T: serde::Serialize>(value: &T) -> Response {
    let mut r = axum::Json(value).into_response();
    r.headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    r
}

/// True when the declared `Content-Length` is over `limit`: such a body is refused before
/// a byte of it is read.
fn declared_too_large(headers: &HeaderMap, limit: usize) -> bool {
    headers
        .get(header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok())
        .is_some_and(|n| n > limit as u64)
}

/// The bearer token from the `Authorization` header only (never a query string).
fn bearer(headers: &HeaderMap) -> Option<[u8; 32]> {
    let mut values = headers.get_all(header::AUTHORIZATION).iter();
    let first = values.next()?;
    if values.next().is_some() {
        return None;
    }
    let token = first.to_str().ok()?.strip_prefix("Bearer ")?;
    unb64_exact::<32>(token)
}

fn log_request(
    endpoint: &'static str,
    start: Instant,
    status: StatusCode,
    error: Option<WireError>,
    dev: Option<&AuthedDevice>,
) {
    let latency = u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);
    let mut fields: Vec<(&'static str, V<'_>)> = vec![
        ("endpoint", V::Name(endpoint)),
        ("status", V::Status(status.as_u16())),
        ("latency_ms", V::N(latency)),
    ];
    if let Some(e) = error {
        fields.push(("error", V::Error(e)));
    }
    if let Some(d) = dev {
        fields.push(("user", V::Id(&d.user_id)));
        fields.push(("device", V::Id(&d.device_ref)));
    }
    logging::log("request", &fields);
}

/// Run `f` on a blocking thread, counted by `server` as work in flight (see
/// [`Server::run_blocking`]).
async fn blocking<T: Send + 'static>(
    server: &Server,
    f: impl FnOnce() -> T + Send + 'static,
) -> Result<T, WireError> {
    server
        .run_blocking(f)
        .await
        .map_err(|_| WireError::CannotRecord)
}

/// Authenticate, answering the error response on failure.
async fn auth(server: &Arc<Server>, headers: &HeaderMap) -> Result<AuthedDevice, WireError> {
    let token = bearer(headers);
    let s = server.clone();
    blocking(server, move || s.authenticate(token.as_ref())).await?
}

/// The router for the LAN listener.
pub fn router(server: Arc<Server>) -> Router {
    Router::new()
        .route("/v1/pair", post(pair))
        .route("/v1/state", get(state))
        .route("/v1/unpair", post(unpair))
        .route("/v1/history", get(history))
        .route("/v1/control", post(control))
        .route("/v1/upload", post(upload))
        .route("/v1/receipt", post(receipt))
        .route("/v1/update/manifest", get(manifest))
        .fallback(not_found)
        .with_state(server)
}

async fn not_found() -> Response {
    error_response(WireError::ProtocolUnsupported)
}

async fn pair(
    State(server): State<Arc<Server>>,
    Extension(Peer(peer)): Extension<Peer>,
    headers: HeaderMap,
    body: Body,
) -> Response {
    let start = Instant::now();
    let floor = server.config.pair_floor;
    let s = server.clone();
    let outcome: Result<_, Refusal> = async {
        let s1 = s.clone();
        blocking(&s, move || s1.count_pair_attempt(peer.ip())).await??;
        if declared_too_large(&headers, PAIR_BODY_LIMIT) {
            return Err(WireError::PayloadTooLarge.into());
        }
        let bytes = to_bytes(body, PAIR_BODY_LIMIT)
            .await
            .map_err(|_| WireError::PayloadTooLarge)?;
        let req: PairRequest =
            serde_json::from_slice(&bytes).map_err(|_| WireError::SchemaInvalid)?;
        let s2 = s.clone();
        blocking(&s, move || s2.redeem(&req, peer.ip())).await?
    }
    .await;
    // One answer time for every outcome (contract 4.4: timing-equalised).
    if let Some(rest) = floor.checked_sub(start.elapsed()) {
        tokio::time::sleep(rest).await;
    }
    match outcome {
        Ok((resp, _token)) => {
            log_request("pair", start, StatusCode::OK, None, None);
            json_response(&resp)
        }
        Err(Refusal::RateLimited(secs)) => {
            log_request(
                "pair",
                start,
                StatusCode::TOO_MANY_REQUESTS,
                Some(WireError::RateLimited),
                None,
            );
            let mut r = error_response(WireError::RateLimited);
            if let Ok(v) = HeaderValue::from_str(&secs.to_string()) {
                r.headers_mut().insert(header::RETRY_AFTER, v);
            }
            r
        }
        Err(Refusal::Wire(e)) => {
            log_request("pair", start, status_of(e), Some(e), None);
            error_response(e)
        }
    }
}

fn after_seq(uri: &Uri) -> u64 {
    uri.query()
        .into_iter()
        .flat_map(|q| q.split('&'))
        .filter_map(|kv| kv.strip_prefix("after_seq="))
        .find_map(|v| v.parse().ok())
        .unwrap_or(0)
}

async fn state(State(server): State<Arc<Server>>, headers: HeaderMap, uri: Uri) -> Response {
    let start = Instant::now();
    let dev = match auth(&server, &headers).await {
        Ok(d) => d,
        Err(e) => {
            log_request("state", start, status_of(e), Some(e), None);
            return error_response(e);
        }
    };
    let after = after_seq(&uri);
    let echo = echo_of(&headers);
    let s = server.clone();
    match blocking(&server, move || {
        s.check_echo(&dev, echo.as_deref())?;
        s.state_for(&dev, after)
    })
    .await
    .and_then(|r| r)
    {
        Ok(state) => {
            log_request("state", start, StatusCode::OK, None, Some(&dev));
            json_response(&state)
        }
        Err(e) => {
            log_request("state", start, status_of(e), Some(e), Some(&dev));
            error_response(e)
        }
    }
}

async fn unpair(State(server): State<Arc<Server>>, headers: HeaderMap) -> Response {
    let start = Instant::now();
    let dev = match auth(&server, &headers).await {
        Ok(d) => d,
        Err(e) => {
            log_request("unpair", start, status_of(e), Some(e), None);
            return error_response(e);
        }
    };
    let s = server.clone();
    match blocking(&server, move || s.unpair(&dev))
        .await
        .and_then(|r| r)
    {
        Ok(()) => {
            log_request("unpair", start, StatusCode::OK, None, Some(&dev));
            json_response(&serde_json::json!({}))
        }
        Err(e) => {
            log_request("unpair", start, status_of(e), Some(e), Some(&dev));
            error_response(e)
        }
    }
}

/// The receipt a device echoes in `HY-Receipt`, if any (one header only).
fn echo_of(headers: &HeaderMap) -> Option<String> {
    let mut values = headers.get_all(RECEIPT_HEADER).iter();
    let first = values.next()?;
    if values.next().is_some() {
        return None;
    }
    first.to_str().ok().map(str::to_owned)
}

fn query_param<'a>(uri: &'a Uri, name: &str) -> Option<&'a str> {
    uri.query()?
        .split('&')
        .filter_map(|kv| kv.split_once('='))
        .find_map(|(k, v)| (k == name).then_some(v))
}

/// The pending-device gate shared by every confirmed-only endpoint: authenticate and
/// refuse a pending device with `device_unconfirmed` before the body is read.
async fn confirmed_only(
    endpoint: &'static str,
    server: &Arc<Server>,
    headers: &HeaderMap,
    start: Instant,
) -> Result<AuthedDevice, Response> {
    let dev = match auth(server, headers).await {
        Ok(d) => d,
        Err(e) => {
            log_request(endpoint, start, status_of(e), Some(e), None);
            return Err(error_response(e));
        }
    };
    if dev.state != DeviceState::Active {
        log_request(
            endpoint,
            start,
            StatusCode::FORBIDDEN,
            Some(WireError::DeviceUnconfirmed),
            Some(&dev),
        );
        return Err(error_response(WireError::DeviceUnconfirmed));
    }
    Ok(dev)
}

async fn read_json<T: serde::de::DeserializeOwned>(
    headers: &HeaderMap,
    body: Body,
    limit: usize,
) -> Result<T, WireError> {
    if declared_too_large(headers, limit) {
        return Err(WireError::PayloadTooLarge);
    }
    let bytes = to_bytes(body, limit)
        .await
        .map_err(|_| WireError::PayloadTooLarge)?;
    serde_json::from_slice(&bytes).map_err(|_| WireError::SchemaInvalid)
}

fn finish<T: serde::Serialize>(
    endpoint: &'static str,
    start: Instant,
    dev: &AuthedDevice,
    result: Result<T, WireError>,
) -> Response {
    match result {
        Ok(v) => {
            log_request(endpoint, start, StatusCode::OK, None, Some(dev));
            json_response(&v)
        }
        Err(e) => {
            log_request(endpoint, start, status_of(e), Some(e), Some(dev));
            error_response(e)
        }
    }
}

async fn history(State(server): State<Arc<Server>>, headers: HeaderMap, uri: Uri) -> Response {
    let start = Instant::now();
    let dev = match confirmed_only("history", &server, &headers, start).await {
        Ok(d) => d,
        Err(r) => return r,
    };
    let echo = echo_of(&headers);
    let from = query_param(&uri, "from").unwrap_or_default().to_owned();
    let to = query_param(&uri, "to").unwrap_or_default().to_owned();
    let s = server.clone();
    let result = blocking(&server, move || {
        s.check_echo(&dev, echo.as_deref())?;
        s.history(&dev, &from, &to)
    })
    .await
    .and_then(|r| r);
    finish("history", start, &dev, result)
}

async fn control(State(server): State<Arc<Server>>, headers: HeaderMap, body: Body) -> Response {
    let start = Instant::now();
    let dev = match confirmed_only("control", &server, &headers, start).await {
        Ok(d) => d,
        Err(r) => return r,
    };
    let echo = echo_of(&headers);
    let result = match read_json::<ControlRequest>(&headers, body, BODY_LIMIT).await {
        Ok(req) => {
            let s = server.clone();
            blocking(&server, move || {
                s.check_echo(&dev, echo.as_deref())?;
                s.control(&dev, &req)
            })
            .await
            .and_then(|r| r)
        }
        Err(e) => Err(e),
    };
    finish("control", start, &dev, result)
}

async fn upload(State(server): State<Arc<Server>>, headers: HeaderMap, body: Body) -> Response {
    let start = Instant::now();
    let dev = match confirmed_only("upload", &server, &headers, start).await {
        Ok(d) => d,
        Err(r) => return r,
    };
    let echo = echo_of(&headers);
    let result = match read_json::<UploadRequest>(&headers, body, BODY_LIMIT).await {
        Ok(req) => {
            let s = server.clone();
            blocking(&server, move || {
                s.check_echo(&dev, echo.as_deref())?;
                s.upload(&dev, &req)
            })
            .await
            .and_then(|r| r)
        }
        Err(e) => Err(e),
    };
    finish("upload", start, &dev, result)
}

/// Body limit for `/v1/receipt` (one head).
pub const RECEIPT_BODY_LIMIT: usize = 4 * 1024;

/// A receipt sent without a usable token: refused, and only checked for a history
/// beyond the server's (never evidence).
async fn refuse_receipt(
    server: Arc<Server>,
    headers: &HeaderMap,
    body: Body,
    e: WireError,
    dev: Option<AuthedDevice>,
    start: Instant,
) -> Response {
    if !declared_too_large(headers, RECEIPT_BODY_LIMIT)
        && let Ok(bytes) = to_bytes(body, RECEIPT_BODY_LIMIT).await
    {
        let s = server.clone();
        let _ = blocking(&server, move || s.receipt_without_device(&bytes)).await;
    }
    log_request("receipt", start, status_of(e), Some(e), dev.as_ref());
    error_response(e)
}

async fn receipt(State(server): State<Arc<Server>>, headers: HeaderMap, body: Body) -> Response {
    let start = Instant::now();
    let dev = match auth(&server, &headers).await {
        Ok(d) => d,
        Err(e) => return refuse_receipt(server, &headers, body, e, None, start).await,
    };
    if dev.state != DeviceState::Active {
        return refuse_receipt(
            server,
            &headers,
            body,
            WireError::DeviceUnconfirmed,
            Some(dev),
            start,
        )
        .await;
    }
    let result = match read_json::<ReceiptRequest>(&headers, body, RECEIPT_BODY_LIMIT).await {
        Ok(req) => {
            let s = server.clone();
            blocking(&server, move || s.present_receipt(&dev, &req.head))
                .await
                .and_then(|r| r)
        }
        Err(e) => Err(e),
    };
    finish("receipt", start, &dev, result)
}

async fn manifest(State(server): State<Arc<Server>>, headers: HeaderMap) -> Response {
    let start = Instant::now();
    let dev = match confirmed_only("manifest", &server, &headers, start).await {
        Ok(d) => d,
        Err(r) => return r,
    };
    // The update manifest arrives in V1-6.
    log_request(
        "manifest",
        start,
        StatusCode::NOT_FOUND,
        Some(WireError::ProtocolUnsupported),
        Some(&dev),
    );
    error_response(WireError::ProtocolUnsupported)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bearer_tokens_come_only_from_one_authorization_header() {
        let token = [7u8; 32];
        let mut h = HeaderMap::new();
        assert_eq!(bearer(&h), None);
        h.insert(
            header::AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {}", hy_core::protocol::b64(&token))).unwrap(),
        );
        assert_eq!(bearer(&h), Some(token));
        h.append(header::AUTHORIZATION, HeaderValue::from_static("Bearer x"));
        assert_eq!(bearer(&h), None, "two headers");
        let mut t = HeaderMap::new();
        t.insert(
            header::AUTHORIZATION,
            HeaderValue::from_str(&format!("Basic {}", hy_core::protocol::b64(&token))).unwrap(),
        );
        assert_eq!(bearer(&t), None);
        let mut short = HeaderMap::new();
        short.insert(
            header::AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {}", hy_core::protocol::b64(&token[..31])))
                .unwrap(),
        );
        assert_eq!(bearer(&short), None);
    }

    #[test]
    fn after_seq_reads_only_its_own_parameter() {
        let u: Uri = "/v1/state?user_id=abc&after_seq=12".parse().unwrap();
        assert_eq!(after_seq(&u), 12);
        let u: Uri = "/v1/state?after_seq=x".parse().unwrap();
        assert_eq!(after_seq(&u), 0);
    }
}
