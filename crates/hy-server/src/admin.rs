//! The admin page (V1-5 contract 4.2, 6.1, TH13 and TH15; architecture 5.1).
//!
//! **Hardening (TH15, F14):**
//! - its own listener on a loopback address only (never the LAN listener);
//! - every request's `Host` must be `127.0.0.1:<port>` or `localhost:<port>` (DNS
//!   rebinding), and every POST's `Origin` must be the same origin (CSRF);
//! - a random 256-bit session cookie (`HttpOnly`, `SameSite=Strict`, `Path=/`, 30-minute
//!   idle timeout), stored only as a hash;
//! - a CSRF token on every form, derived from the session cookie;
//! - an Argon2id passphrase hash with exponential backoff after failed sign-ins;
//! - `Content-Security-Policy: default-src 'none'`, no framing, no caching.
//!
//! **No habit data.** No admin page or endpoint returns a summary, a payload or any
//! aggregate. The pages show users (aliases), devices (labels, states, check codes), the
//! server's fingerprint and six words, slips once, and the audit log.
//!
//! **Confirmation is the only way a device becomes active** ([`Server::confirm_device`]),
//! and it needs the check code the user read out, typed in.

use std::fmt::Write as _;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use argon2::{Algorithm, Argon2, Params, Version};
use axum::Router;
use axum::extract::{Form, Request, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use chrono::DateTime;
use hy_core::protocol::words::fingerprint_phrase;
use hy_core::protocol::{b64, ct_eq32, hex, sha256, unb64_exact};
use hy_core::secret::Secret;
use rusqlite::{OptionalExtension, params};
use serde::Deserialize;
use subtle::ConstantTimeEq;

use crate::audit::{self, AuditEvent};
use crate::server::{AdminError, DeviceState, SESSION_IDLE_MS, Server, UNSEEN_FLAG_MS};

const COOKIE: &str = "hy_admin";

/// Longest sign-in backoff.
pub const MAX_BACKOFF_S: i64 = 15 * 60;

/// Hash a passphrase with Argon2id: `argon2id$m=<KiB>,t=<passes>,p=<lanes>$<salt>$<hash>`.
#[must_use]
pub fn hash_passphrase(
    passphrase: &Secret<String>,
    salt: &[u8; 16],
    (m, t, p): (u32, u32, u32),
) -> Option<String> {
    let params = Params::new(m, t, p, Some(32)).ok()?;
    let mut out = [0u8; 32];
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
        .hash_password_into(passphrase.expose().as_bytes(), salt, &mut out)
        .ok()?;
    Some(format!(
        "argon2id$m={m},t={t},p={p}${}${}",
        b64(salt),
        b64(&out)
    ))
}

/// Check a passphrase against a stored hash, in constant time.
#[must_use]
pub fn verify_passphrase(stored: &str, passphrase: &Secret<String>) -> bool {
    let mut parts = stored.split('$');
    let (Some("argon2id"), Some(cost), Some(salt), Some(hash), None) = (
        parts.next(),
        parts.next(),
        parts.next(),
        parts.next(),
        parts.next(),
    ) else {
        return false;
    };
    let mut m = 0;
    let mut t = 0;
    let mut p = 0;
    for kv in cost.split(',') {
        match kv.split_once('=') {
            Some(("m", v)) => m = v.parse().unwrap_or(0),
            Some(("t", v)) => t = v.parse().unwrap_or(0),
            Some(("p", v)) => p = v.parse().unwrap_or(0),
            _ => return false,
        }
    }
    let (Some(salt), Some(expected)) = (unb64_exact::<16>(salt), unb64_exact::<32>(hash)) else {
        return false;
    };
    let Some(again) = hash_passphrase(passphrase, &salt, (m, t, p)) else {
        return false;
    };
    let again = again.rsplit('$').next().and_then(unb64_exact::<32>);
    again.is_some_and(|a| ct_eq32(&a, &expected))
}

#[derive(Debug, Clone)]
struct AdminState {
    server: Arc<Server>,
    port: u16,
}

/// The admin router for a loopback listener on `port`.
pub fn router(server: Arc<Server>, port: u16) -> Router {
    let state = AdminState { server, port };
    Router::new()
        .route("/", get(dashboard))
        .route("/login", get(login_page).post(login))
        .route("/logout", post(logout))
        .route("/users", post(create_user))
        .route("/users/reject-pending", post(reject_pending))
        .route("/slips", post(new_slip))
        .route("/devices/confirm", post(confirm))
        .route("/devices/revoke", post(revoke))
        .route("/recovery/users", post(witness_users))
        .route("/recovery/user", post(witness_user))
        .fallback(|| async { (StatusCode::NOT_FOUND, "Not found") })
        .layer(middleware::from_fn_with_state(state.clone(), guard))
        .with_state(state)
}

fn allowed_origins(port: u16) -> [String; 2] {
    [
        format!("http://127.0.0.1:{port}"),
        format!("http://localhost:{port}"),
    ]
}

/// Host and Origin checks, and the security headers on every response.
async fn guard(State(st): State<AdminState>, req: Request, next: Next) -> Response {
    let hosts = [
        format!("127.0.0.1:{}", st.port),
        format!("localhost:{}", st.port),
    ];
    let host_ok = req
        .headers()
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .is_some_and(|h| hosts.iter().any(|a| a == h));
    let mut resp = if !host_ok {
        (
            StatusCode::MISDIRECTED_REQUEST,
            "Use http://127.0.0.1 on the server PC.",
        )
            .into_response()
    } else if req.method() == axum::http::Method::POST
        && !req
            .headers()
            .get(header::ORIGIN)
            .and_then(|h| h.to_str().ok())
            .is_some_and(|o| allowed_origins(st.port).iter().any(|a| a == o))
    {
        (StatusCode::FORBIDDEN, "Cross-origin request refused.").into_response()
    } else {
        next.run(req).await
    };
    let h = resp.headers_mut();
    h.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(
            "default-src 'none'; style-src 'unsafe-inline'; form-action 'self'; frame-ancestors 'none'; base-uri 'none'",
        ),
    );
    h.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    h.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    h.insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    resp
}

/// HTML-escape text (aliases and labels come from people and devices).
#[must_use]
pub fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

fn page(title: &str, body: &str) -> Response {
    let html = format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><title>{} - HealthierYou server</title>\
         <style>body{{font:15px/1.45 system-ui,sans-serif;margin:24px;max-width:1100px;color:#1d2433}}\
         table{{border-collapse:collapse;margin:8px 0 20px}}td,th{{border:1px solid #c9ced8;padding:4px 8px;text-align:left;vertical-align:top}}\
         code,.mono{{font-family:ui-monospace,Consolas,monospace}}.warn{{background:#fff4d6;padding:8px;border:1px solid #e0b84d}}\
         .slip{{font-size:18px;word-break:break-all;background:#f3f5f9;padding:12px}}h2{{margin-top:28px}}form{{display:inline}}</style></head>\
         <body><h1>HealthierYou office server</h1>{}</body></html>",
        esc(title),
        body
    );
    (
        [(
            header::CONTENT_TYPE,
            HeaderValue::from_static("text/html; charset=utf-8"),
        )],
        html,
    )
        .into_response()
}

fn cookie_value(headers: &HeaderMap) -> Option<[u8; 32]> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|kv| {
            kv.trim()
                .strip_prefix(&format!("{COOKIE}="))
                .map(str::to_owned)
        })
        .find_map(|v| unb64_exact::<32>(&v))
}

fn session_hash(cookie: &[u8; 32]) -> [u8; 32] {
    sha256(&[b"HY-ADMINSESSION1", cookie])
}

fn csrf_for(cookie: &[u8; 32]) -> String {
    hex(&sha256(&[b"HY-CSRF1", cookie]))
}

/// The session behind the request's cookie, refreshed; `None` if absent or idle too long.
fn session(server: &Server, headers: &HeaderMap) -> Option<[u8; 32]> {
    let cookie = cookie_value(headers)?;
    let now = server.clock.now().timestamp_millis();
    let conn = server.db();
    let last: Option<i64> = conn
        .query_row(
            "SELECT last_used FROM admin_sessions WHERE session_hash = ?1",
            [session_hash(&cookie).as_slice()],
            |r| r.get(0),
        )
        .optional()
        .ok()
        .flatten();
    let last = last?;
    if now - last > SESSION_IDLE_MS {
        let _ = conn.execute(
            "DELETE FROM admin_sessions WHERE session_hash = ?1",
            [session_hash(&cookie).as_slice()],
        );
        return None;
    }
    let _ = conn.execute(
        "UPDATE admin_sessions SET last_used = ?2 WHERE session_hash = ?1",
        params![session_hash(&cookie).as_slice(), now],
    );
    Some(cookie)
}

fn csrf_ok(cookie: &[u8; 32], submitted: &str) -> bool {
    csrf_for(cookie)
        .as_bytes()
        .ct_eq(submitted.as_bytes())
        .into()
}

fn to_login() -> Response {
    Redirect::to("/login").into_response()
}

fn when(ms: i64) -> String {
    DateTime::from_timestamp_millis(ms).map_or_else(String::new, |t| {
        t.with_timezone(&chrono_tz::Asia::Kolkata)
            .format("%Y-%m-%d %H:%M IST")
            .to_string()
    })
}

fn short(id: &[u8]) -> String {
    hex(&id[..id.len().min(4)])
}

fn parse_id(text: &str) -> Option<[u8; 16]> {
    let bytes = data_encoding::HEXLOWER_PERMISSIVE
        .decode(text.as_bytes())
        .ok()?;
    bytes.try_into().ok()
}

// ------------------------------------------------------------------ sign-in

async fn login_page(State(st): State<AdminState>) -> Response {
    let has_pass: bool = st
        .server
        .db()
        .query_row("SELECT EXISTS (SELECT 1 FROM admin)", [], |r| r.get(0))
        .unwrap_or(false);
    if !has_pass {
        return page(
            "Set a passphrase",
            "<p class=\"warn\">No admin passphrase is set. On this PC, run <code>hy-server set-passphrase</code> as an administrator, then reload this page.</p>",
        );
    }
    page(
        "Sign in",
        "<h2>Sign in</h2><form method=\"post\" action=\"/login\"><label>Admin passphrase \
         <input type=\"password\" name=\"passphrase\" autocomplete=\"current-password\" required></label> \
         <button>Sign in</button></form>",
    )
}

#[derive(Deserialize)]
struct LoginForm {
    passphrase: String,
}

fn too_many(wait_s: i64) -> Response {
    (
        StatusCode::TOO_MANY_REQUESTS,
        page(
            "Wait",
            &format!("<p class=\"warn\">Too many failed sign-ins. Try again in {wait_s} s.</p>"),
        ),
    )
        .into_response()
}

/// What one sign-in attempt came to.
enum LoginOutcome {
    /// No passphrase is set.
    NoAdmin,
    /// Another check is running, or the backoff is in force: wait this many seconds.
    Wait(i64),
    /// The passphrase is wrong (already counted).
    Wrong,
    /// Signed in with this new session cookie (already stored).
    Session([u8; 32]),
    /// The random source failed: nothing was issued.
    NoRandom,
}

/// One whole sign-in attempt, run on a blocking thread: the permit, the fresh read of the
/// lock, the Argon2 check and the recorded outcome all happen here, so cancelling the
/// request (a client that goes away, the connection timeout) can neither free the permit
/// while a check still runs nor skip counting a failure.
fn attempt_login(server: &Server, pass: &Secret<String>) -> LoginOutcome {
    let Ok(_permit) = server.login_gate.try_acquire() else {
        return LoginOutcome::Wait(1);
    };
    let now = server.clock.now().timestamp_millis();
    let row: Option<(String, i64)> = server
        .db()
        .query_row(
            "SELECT passphrase_hash, locked_until FROM admin WHERE id = 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .unwrap_or(None);
    let Some((stored, locked_until)) = row else {
        return LoginOutcome::NoAdmin;
    };
    if now < locked_until {
        return LoginOutcome::Wait((locked_until - now + 999) / 1000);
    }
    let (running, peak) = &server.login_checks;
    let n = running.fetch_add(1, Ordering::SeqCst) + 1;
    peak.fetch_max(n, Ordering::SeqCst);
    let ok = verify_passphrase(&stored, pass);
    running.fetch_sub(1, Ordering::SeqCst);
    let conn = server.db();
    if !ok {
        // Counted from the stored value, inside the permit.
        let _ = conn.execute(
            "UPDATE admin SET failures = failures + 1,
                 locked_until = ?1 + 1000 * min(?2, 1 << min(failures, 10))
             WHERE id = 1",
            params![now, MAX_BACKOFF_S],
        );
        let _ = audit::record(&conn, now, AuditEvent::AdminLoginFailed, None, None, None);
        return LoginOutcome::Wrong;
    }
    let mut cookie = [0u8; 32];
    if getrandom::fill(&mut cookie).is_err() {
        return LoginOutcome::NoRandom;
    }
    let _ = conn.execute(
        "UPDATE admin SET failures = 0, locked_until = 0 WHERE id = 1",
        [],
    );
    let _ = conn.execute(
        "INSERT INTO admin_sessions (session_hash, created_at, last_used) VALUES (?1, ?2, ?2)",
        params![session_hash(&cookie).as_slice(), now],
    );
    let _ = audit::record(&conn, now, AuditEvent::AdminLogin, None, None, None);
    LoginOutcome::Session(cookie)
}

/// Sign in. Only one passphrase check runs at a time, process-wide: a sign-in that finds
/// one running is refused at once (never queued), so concurrent wrong attempts can
/// neither run many 64 MiB Argon2 jobs nor all read the same failure count. The whole
/// attempt, permit included, runs in [`attempt_login`] on a blocking thread, which
/// finishes and records its outcome even if this request is cancelled.
async fn login(State(st): State<AdminState>, Form(form): Form<LoginForm>) -> Response {
    let pass = Secret::new(form.passphrase);
    let s = st.server.clone();
    let outcome = st
        .server
        .run_blocking(move || attempt_login(&s, &pass))
        .await
        .unwrap_or(LoginOutcome::Wait(1));
    match outcome {
        LoginOutcome::NoAdmin => to_login(),
        LoginOutcome::Wait(s) => too_many(s),
        LoginOutcome::Wrong => (
            StatusCode::UNAUTHORIZED,
            page("Sign in", "<p class=\"warn\">That passphrase is not right.</p><p><a href=\"/login\">Try again</a></p>"),
        )
            .into_response(),
        LoginOutcome::NoRandom => {
            (StatusCode::SERVICE_UNAVAILABLE, "random source failed").into_response()
        }
        LoginOutcome::Session(cookie) => {
            let mut r = Redirect::to("/").into_response();
            if let Ok(v) = HeaderValue::from_str(&format!(
                "{COOKIE}={}; HttpOnly; SameSite=Strict; Path=/; Max-Age=1800",
                b64(&cookie)
            )) {
                r.headers_mut().insert(header::SET_COOKIE, v);
            }
            r
        }
    }
}

#[derive(Deserialize)]
struct CsrfOnly {
    csrf: String,
}

async fn logout(
    State(st): State<AdminState>,
    headers: HeaderMap,
    Form(f): Form<CsrfOnly>,
) -> Response {
    let Some(cookie) = session(&st.server, &headers) else {
        return to_login();
    };
    if !csrf_ok(&cookie, &f.csrf) {
        return (StatusCode::FORBIDDEN, "Form expired. Reload the page.").into_response();
    }
    let now = st.server.clock.now().timestamp_millis();
    let conn = st.server.db();
    let _ = conn.execute(
        "DELETE FROM admin_sessions WHERE session_hash = ?1",
        [session_hash(&cookie).as_slice()],
    );
    let _ = audit::record(&conn, now, AuditEvent::AdminLogout, None, None, None);
    let mut r = Redirect::to("/login").into_response();
    r.headers_mut().insert(
        header::SET_COOKIE,
        HeaderValue::from_static("hy_admin=; HttpOnly; SameSite=Strict; Path=/; Max-Age=0"),
    );
    r
}

// ---------------------------------------------------------------- dashboard

fn hidden_csrf(cookie: &[u8; 32]) -> String {
    format!(
        "<input type=\"hidden\" name=\"csrf\" value=\"{}\">",
        csrf_for(cookie)
    )
}

async fn dashboard(State(st): State<AdminState>, headers: HeaderMap) -> Response {
    let Some(cookie) = session(&st.server, &headers) else {
        return to_login();
    };
    let s = &st.server;
    let (Ok(users), Ok(devices)) = (s.users(), s.devices()) else {
        return page(
            "Error",
            "<p class=\"warn\">The server database could not be read.</p>",
        );
    };
    let now = s.clock.now().timestamp_millis();
    let csrf = hidden_csrf(&cookie);
    let alias_of = |id: &[u8]| {
        users
            .iter()
            .find(|u| u.user_id.as_slice() == id)
            .map_or_else(|| short(id), |u| u.alias.clone())
    };
    let fp = hex(&s.keys.fingerprint);
    let grouped: Vec<String> = fp
        .as_bytes()
        .chunks(8)
        .map(|c| String::from_utf8_lossy(c).into_owned())
        .collect();
    let mut b = String::new();
    let _ = write!(
        b,
        "<form method=\"post\" action=\"/logout\">{csrf}<button>Sign out</button></form>\
         <h2>This server</h2><p>Fingerprint (SHA-256 of the certificate): <span class=\"mono\">{}</span></p>\
         <p>Six words: <strong class=\"mono\">{}</strong></p>",
        grouped.join(" "),
        esc(&fingerprint_phrase(&s.keys.fingerprint))
    );
    let mirror = if s.mirror_open() {
        "open and written with every change"
    } else {
        "<strong class=\"warn\">not available: control changes are refused until it is back</strong>"
    };
    let backup = match crate::backup::last_backup(s).ok().flatten() {
        Some(at) => when(at),
        // A restored database was copied before its own backup was recorded.
        None if crate::backup::is_restored_copy(s) => {
            "No backup recorded in this restored copy".to_owned()
        }
        None => "none yet".to_owned(),
    };
    let _ = write!(
        b,
        "<p>Control-log mirror: {mirror}.</p><p>Last backup: {backup}.</p>\
         <p class=\"note\">Backups open only with this server's backup key (<span class=\"mono\">backup.key</span> \
         in the keys folder, protected for this PC). If the key is lost, every existing backup is permanently unrecoverable. \
         A copy of the keys folder opens only on this same Windows installation: it is not a way to move or rebuild the server. \
         On a replacement server, after a Windows reinstall, or if the keys cannot be opened, the only route is a full reset: \
         new keys, every PC re-pairs with a new slip, and the old backups cannot be opened.</p>"
    );
    if users.iter().any(|u| u.hold) {
        b.push_str("<p class=\"warn\">One or more users are in recovery hold: their shared history is paused.</p>");
        recovery_section(&mut b, s, &csrf, now);
    }

    b.push_str(
        "<h2>PCs waiting for confirmation</h2>\
         <p>Confirm a PC only when its user, in person or on a call <em>you</em> place to their known extension, reads out \
         from that PC the six words (they must equal the six words above) and the check code. If the words differ, reject it: \
         the slip may have been altered. If the user says their slip did not work, reject every waiting PC for that user.</p>",
    );
    let pending: Vec<_> = devices
        .iter()
        .filter(|d| d.state == DeviceState::Pending)
        .collect();
    if pending.is_empty() {
        b.push_str("<p>None.</p>");
    } else {
        b.push_str("<table><tr><th>User</th><th>PC label</th><th>Paired</th><th>Check code</th><th>Confirm</th><th></th></tr>");
        for d in pending {
            let id = hex(&d.device_ref);
            let _ = write!(
                b,
                "<tr><td>{}</td><td>{}</td><td>{}</td><td class=\"mono\"><strong>{}</strong></td>\
                 <td><form method=\"post\" action=\"/devices/confirm\">{csrf}<input type=\"hidden\" name=\"device\" value=\"{id}\">\
                 <label>Code read out <input name=\"check_code\" size=\"5\" maxlength=\"9\" required></label> \
                 <label><input type=\"checkbox\" name=\"words_match\" value=\"yes\" required> The six words match</label> \
                 <button>Confirm</button></form></td>\
                 <td><form method=\"post\" action=\"/devices/revoke\">{csrf}<input type=\"hidden\" name=\"device\" value=\"{id}\"><button>Reject</button></form></td></tr>",
                esc(&alias_of(&d.user_id)),
                esc(&d.label),
                when(d.created_at),
                esc(&d.check_code),
            );
        }
        b.push_str("</table>");
    }

    b.push_str("<h2>Users</h2>");
    let _ = write!(
        b,
        "<form method=\"post\" action=\"/users\">{csrf}<label>New user alias <input name=\"alias\" maxlength=\"40\" required></label> <button>Create</button></form>"
    );
    b.push_str("<table><tr><th>Alias</th><th>PCs</th><th>Slips</th><th></th></tr>");
    for u in &users {
        let uid = hex(&u.user_id);
        let mut pcs = String::new();
        for d in devices.iter().filter(|d| d.user_id == u.user_id) {
            let state = match d.state {
                DeviceState::Pending => "waiting".to_owned(),
                DeviceState::Active => {
                    format!("confirmed {}", d.confirmed_at.map(when).unwrap_or_default())
                }
                DeviceState::Revoked => format!(
                    "revoked ({})",
                    esc(d.revoke_reason.as_deref().unwrap_or(""))
                ),
            };
            let seen = d.last_seen.map_or_else(|| "never".to_owned(), when);
            let regressed = d
                .regressed_at
                .filter(|_| d.state == DeviceState::Active)
                .map(|at| {
                    format!(
                        " <strong>(presented an older receipt {}: a restored or copied PC)</strong>",
                        when(at)
                    )
                })
                .unwrap_or_default();
            let unseen = d.state == DeviceState::Active
                && now - d.last_seen.unwrap_or(d.created_at) > UNSEEN_FLAG_MS;
            let _ = write!(
                pcs,
                "<div>{} - {state}; last seen {seen}{}{regressed}",
                esc(&d.label),
                if unseen {
                    " <strong>(unseen for over 14 days)</strong>"
                } else {
                    ""
                }
            );
            if d.state == DeviceState::Active {
                let _ = write!(
                    pcs,
                    " <form method=\"post\" action=\"/devices/revoke\">{csrf}<input type=\"hidden\" name=\"device\" value=\"{}\"><button>Revoke</button></form>",
                    hex(&d.device_ref)
                );
            }
            pcs.push_str("</div>");
        }
        let _ = write!(
            b,
            "<tr><td>{}{}</td><td>{}</td><td>{} outstanding</td><td>\
             <form method=\"post\" action=\"/slips\">{csrf}<input type=\"hidden\" name=\"user\" value=\"{uid}\"><button>New slip</button></form> \
             <form method=\"post\" action=\"/users/reject-pending\">{csrf}<input type=\"hidden\" name=\"user\" value=\"{uid}\"><button>Reject all waiting PCs</button></form></td></tr>",
            esc(&u.alias),
            if u.hold { " (recovery hold)" } else { "" },
            if pcs.is_empty() {
                "none".to_owned()
            } else {
                pcs
            },
            u.outstanding_slips,
        );
    }
    b.push_str("</table>");

    b.push_str("<h2>Audit log (last 50)</h2><table><tr><th>When</th><th>Event</th><th>User</th><th>PC</th><th>Network</th></tr>");
    let rows = audit::recent(&s.db(), 50).unwrap_or_default();
    for r in rows {
        let _ = write!(
            b,
            "<tr><td>{}</td><td>{}</td><td>{}</td><td class=\"mono\">{}</td><td class=\"mono\">{}</td></tr>",
            when(r.at),
            esc(&r.event),
            r.user_id
                .as_deref()
                .map(|u| esc(&alias_of(u)))
                .unwrap_or_default(),
            r.device_ref.as_deref().map(short).unwrap_or_default(),
            esc(r.net24.as_deref().unwrap_or("")),
        );
    }
    b.push_str("</table>");
    page("Overview", &b)
}

/// The recovery-hold section: what each held user still needs, and the witness forms
/// (contract 9). Elapsed time is never evidence, so no form releases a user directly.
fn recovery_section(b: &mut String, s: &Server, csrf: &str, now: i64) {
    let Ok(rows) = s.hold_rows() else {
        b.push_str("<p class=\"warn\">The recovery state could not be read.</p>");
        return;
    };
    b.push_str(
        "<h2>Recovery hold</h2>\
         <p>The database was restored or rolled back. A user is released only when every one of their confirmed PCs \
         has checked in with its own token and you have confirmed both the list of users and that user's PCs. \
         A user still incomplete after 7 days has their shared history discarded; time alone never releases anyone.</p>",
    );
    if s.users_witnessed() {
        b.push_str("<p>The list of users: confirmed.</p>");
    } else {
        let _ = write!(
            b,
            "<p>Is the list of users below complete and correct, with no user missing or added?</p>\
             <form method=\"post\" action=\"/recovery/users\">{csrf}<input type=\"hidden\" name=\"right\" value=\"yes\"><button>Yes, the list is right</button></form> \
             <form method=\"post\" action=\"/recovery/users\">{csrf}<input type=\"hidden\" name=\"right\" value=\"no\"><label><input type=\"checkbox\" name=\"sure\" value=\"yes\" required> I understand this cannot be undone</label> <button>No: discard every user's shared history</button></form>"
        );
    }
    b.push_str("<table><tr><th>User</th><th>Hold since</th><th>Discarded at</th><th>PCs (check-in)</th><th>Your confirmation</th></tr>");
    for r in rows {
        let uid = hex(&r.user_id);
        let mut pcs = String::new();
        for (dev, label, state, seq) in &r.devices {
            let status = match (state, seq) {
                (DeviceState::Pending, _) => {
                    "<strong>waiting for confirmation</strong>: it can never check in; reject it (the user re-pairs after release)"
                        .to_owned()
                }
                (_, Some(seq)) => format!("checked in (entry {seq})"),
                (_, None) => "<strong>missing</strong>".to_owned(),
            };
            let _ = write!(
                pcs,
                "<div>{} <span class=\"mono\">{}</span>: {status}</div>",
                esc(label),
                short(dev)
            );
        }
        let witness = if r.witnessed {
            "confirmed".to_owned()
        } else {
            format!(
                "<form method=\"post\" action=\"/recovery/user\">{csrf}<input type=\"hidden\" name=\"user\" value=\"{uid}\"><input type=\"hidden\" name=\"right\" value=\"yes\"><button>These are all their PCs</button></form> \
                 <form method=\"post\" action=\"/recovery/user\">{csrf}<input type=\"hidden\" name=\"user\" value=\"{uid}\"><input type=\"hidden\" name=\"right\" value=\"no\"><label><input type=\"checkbox\" name=\"sure\" value=\"yes\" required> I understand</label> <button>Not right: discard</button></form>"
            )
        };
        let deadline = r.started_at + crate::recovery::HOLD_MAX_MS;
        let _ = write!(
            b,
            "<tr><td>{}</td><td>{}</td><td>{}{}</td><td>{}</td><td>{witness}</td></tr>",
            esc(&r.alias),
            when(r.started_at),
            when(deadline),
            if deadline - now < 24 * 3600 * 1000 {
                " <strong>(within a day)</strong>"
            } else {
                ""
            },
            if pcs.is_empty() {
                "none".to_owned()
            } else {
                pcs
            },
        );
    }
    b.push_str("</table>");
}

#[derive(Deserialize)]
struct WitnessForm {
    csrf: String,
    right: String,
    #[serde(default)]
    user: String,
    /// Required for a "not right" answer, which discards.
    #[serde(default)]
    sure: String,
}

impl WitnessForm {
    /// The answer: yes, or a confirmed no. Anything else changes nothing.
    fn answer(&self) -> Option<bool> {
        match (self.right.as_str(), self.sure.as_str()) {
            ("yes", _) => Some(true),
            ("no", "yes") => Some(false),
            _ => None,
        }
    }
}

fn not_confirmed() -> Response {
    (
        StatusCode::BAD_REQUEST,
        page("Not done", "<p class=\"warn\">Tick the box to confirm a discard. Nothing was changed.</p><p><a href=\"/\">Back</a></p>"),
    )
        .into_response()
}

async fn witness_users(
    State(st): State<AdminState>,
    headers: HeaderMap,
    Form(f): Form<WitnessForm>,
) -> Response {
    let Some(cookie) = session(&st.server, &headers) else {
        return to_login();
    };
    if !csrf_ok(&cookie, &f.csrf) {
        return (StatusCode::FORBIDDEN, "Form expired. Reload the page.").into_response();
    }
    let Some(right) = f.answer() else {
        return not_confirmed();
    };
    match st.server.witness_users(right) {
        Ok(()) => Redirect::to("/").into_response(),
        Err(e) => admin_error(e),
    }
}

async fn witness_user(
    State(st): State<AdminState>,
    headers: HeaderMap,
    Form(f): Form<WitnessForm>,
) -> Response {
    let Some(cookie) = session(&st.server, &headers) else {
        return to_login();
    };
    if !csrf_ok(&cookie, &f.csrf) {
        return (StatusCode::FORBIDDEN, "Form expired. Reload the page.").into_response();
    }
    let Some(user) = parse_id(&f.user) else {
        return admin_error(AdminError::NotFound);
    };
    let Some(right) = f.answer() else {
        return not_confirmed();
    };
    match st.server.witness_user(&user, right) {
        Ok(()) => Redirect::to("/").into_response(),
        Err(e) => admin_error(e),
    }
}

fn admin_error(e: AdminError) -> Response {
    let status = match e {
        AdminError::NotFound => StatusCode::NOT_FOUND,
        AdminError::Storage | AdminError::NoRandom => StatusCode::SERVICE_UNAVAILABLE,
        _ => StatusCode::CONFLICT,
    };
    (
        status,
        page(
            "Not done",
            &format!(
                "<p class=\"warn\">{}</p><p><a href=\"/\">Back</a></p>",
                esc(&e.to_string())
            ),
        ),
    )
        .into_response()
}

#[derive(Deserialize)]
struct AliasForm {
    csrf: String,
    alias: String,
}

async fn create_user(
    State(st): State<AdminState>,
    headers: HeaderMap,
    Form(f): Form<AliasForm>,
) -> Response {
    let Some(cookie) = session(&st.server, &headers) else {
        return to_login();
    };
    if !csrf_ok(&cookie, &f.csrf) {
        return (StatusCode::FORBIDDEN, "Form expired. Reload the page.").into_response();
    }
    match st.server.create_user(&f.alias) {
        Ok(_) => Redirect::to("/").into_response(),
        Err(e) => admin_error(e),
    }
}

#[derive(Deserialize)]
struct UserForm {
    csrf: String,
    user: String,
}

async fn new_slip(
    State(st): State<AdminState>,
    headers: HeaderMap,
    Form(f): Form<UserForm>,
) -> Response {
    let Some(cookie) = session(&st.server, &headers) else {
        return to_login();
    };
    if !csrf_ok(&cookie, &f.csrf) {
        return (StatusCode::FORBIDDEN, "Form expired. Reload the page.").into_response();
    }
    let Some(user) = parse_id(&f.user) else {
        return admin_error(AdminError::NotFound);
    };
    let slip = match st.server.issue_slip(&user) {
        Ok(s) => s,
        Err(e) => return admin_error(e),
    };
    let alias = st
        .server
        .users()
        .ok()
        .and_then(|us| us.into_iter().find(|u| u.user_id == user))
        .map(|u| u.alias)
        .unwrap_or_default();
    let body = format!(
        "<h2>Pairing slip for {}</h2>\
         <p class=\"warn\">This slip is shown <strong>once</strong>. It is valid for 24 hours and works once. \
         Copy it into internal chat, print it, or read it out. It does not name the user.</p>\
         <p>To copy:</p><p class=\"slip mono\">{}</p>\
         <p>To print or type (groups of four):</p><p class=\"slip mono\">{}</p>\
         <p>Six words (the user's PC must show the same words): <strong class=\"mono\">{}</strong></p>\
         <p><a href=\"/\">Back to the overview</a></p>",
        esc(&alias),
        esc(slip.canonical().expose()),
        esc(slip.printable().expose()),
        esc(&fingerprint_phrase(&slip.fingerprint)),
    );
    page("Pairing slip", &body)
}

async fn reject_pending(
    State(st): State<AdminState>,
    headers: HeaderMap,
    Form(f): Form<UserForm>,
) -> Response {
    let Some(cookie) = session(&st.server, &headers) else {
        return to_login();
    };
    if !csrf_ok(&cookie, &f.csrf) {
        return (StatusCode::FORBIDDEN, "Form expired. Reload the page.").into_response();
    }
    let Some(user) = parse_id(&f.user) else {
        return admin_error(AdminError::NotFound);
    };
    match st.server.reject_pending_for_user(&user) {
        Ok(_) => Redirect::to("/").into_response(),
        Err(e) => admin_error(e),
    }
}

#[derive(Deserialize)]
struct ConfirmForm {
    csrf: String,
    device: String,
    check_code: String,
    words_match: Option<String>,
}

async fn confirm(
    State(st): State<AdminState>,
    headers: HeaderMap,
    Form(f): Form<ConfirmForm>,
) -> Response {
    let Some(cookie) = session(&st.server, &headers) else {
        return to_login();
    };
    if !csrf_ok(&cookie, &f.csrf) {
        return (StatusCode::FORBIDDEN, "Form expired. Reload the page.").into_response();
    }
    if f.words_match.as_deref() != Some("yes") {
        return (
            StatusCode::BAD_REQUEST,
            page("Not done", "<p class=\"warn\">Tick \"The six words match\" only after comparing them. Nothing was changed.</p><p><a href=\"/\">Back</a></p>"),
        )
            .into_response();
    }
    let Some(device) = parse_id(&f.device) else {
        return admin_error(AdminError::NotFound);
    };
    match st.server.confirm_device(&device, &f.check_code) {
        Ok(()) => Redirect::to("/").into_response(),
        Err(e) => admin_error(e),
    }
}

#[derive(Deserialize)]
struct DeviceForm {
    csrf: String,
    device: String,
}

async fn revoke(
    State(st): State<AdminState>,
    headers: HeaderMap,
    Form(f): Form<DeviceForm>,
) -> Response {
    let Some(cookie) = session(&st.server, &headers) else {
        return to_login();
    };
    if !csrf_ok(&cookie, &f.csrf) {
        return (StatusCode::FORBIDDEN, "Form expired. Reload the page.").into_response();
    }
    let Some(device) = parse_id(&f.device) else {
        return admin_error(AdminError::NotFound);
    };
    match st.server.revoke_device(&device) {
        Ok(()) => Redirect::to("/").into_response(),
        Err(e) => admin_error(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passphrases_verify_only_with_the_right_text_and_parameters() {
        let p = Secret::new("correct horse battery".to_owned());
        let stored = hash_passphrase(&p, &[1; 16], (64, 1, 1)).unwrap();
        assert!(stored.starts_with("argon2id$m=64,t=1,p=1$"));
        assert!(verify_passphrase(&stored, &p));
        assert!(!verify_passphrase(
            &stored,
            &Secret::new("correct horse battery!".into())
        ));
        assert!(!verify_passphrase("argon2id$m=64$x$y", &p));
        assert!(!verify_passphrase(&stored.replace("t=1", "t=2"), &p));
        assert!(!stored.contains("correct"));
    }

    #[test]
    fn html_is_escaped() {
        assert_eq!(
            esc("<b a='x'>&\"</b>"),
            "&lt;b a=&#39;x&#39;&gt;&amp;&quot;&lt;/b&gt;"
        );
    }

    #[test]
    fn csrf_tokens_are_bound_to_the_session() {
        let a = [1u8; 32];
        let b = [2u8; 32];
        assert!(csrf_ok(&a, &csrf_for(&a)));
        assert!(!csrf_ok(&a, &csrf_for(&b)));
        assert!(!csrf_ok(&a, ""));
    }
}
