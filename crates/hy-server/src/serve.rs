//! The listeners (V1-5 contract 5 and TH15).
//!
//! - **LAN API**: TLS 1.3 accept loop. At most 64 concurrent connections and 8 per source
//!   IP; a 10 s limit to finish the TLS handshake and to send each request's headers; a
//!   2-minute cap on any one connection; ALPN must be `hy/1` or the connection is closed;
//!   HTTP/1.1 only.
//! - **Admin page**: plain HTTP on a loopback address only (a non-loopback address is
//!   refused), served by [`crate::admin`].

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use hy_core::protocol::ALPN;
use hyper_util::rt::{TokioIo, TokioTimer};
use hyper_util::service::TowerToHyperService;
use tokio::net::TcpListener;
use tokio::sync::watch;
use tokio_rustls::TlsAcceptor;
use tower::ServiceExt;

use crate::api::{self, Peer};
use crate::logging::{self, V};
use crate::server::Server;

/// Connection limits.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// Concurrent connections in total.
    pub max_connections: usize,
    /// Concurrent connections per source IP.
    pub max_per_ip: usize,
    /// Time to finish the TLS handshake.
    pub handshake_timeout: Duration,
    /// Time to send each request's headers (also the keep-alive idle limit).
    pub header_timeout: Duration,
    /// Longest life of one connection.
    pub connection_cap: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_connections: 64,
            max_per_ip: 8,
            handshake_timeout: Duration::from_secs(10),
            header_timeout: Duration::from_secs(10),
            connection_cap: Duration::from_secs(120),
        }
    }
}

#[derive(Debug, Default)]
struct Counts {
    total: usize,
    per_ip: HashMap<IpAddr, usize>,
}

#[derive(Debug)]
struct Slot {
    counts: Arc<Mutex<Counts>>,
    ip: IpAddr,
}

impl Drop for Slot {
    fn drop(&mut self) {
        let mut c = self.counts.lock().unwrap_or_else(PoisonError::into_inner);
        c.total = c.total.saturating_sub(1);
        if let Some(n) = c.per_ip.get_mut(&self.ip) {
            *n = n.saturating_sub(1);
            if *n == 0 {
                c.per_ip.remove(&self.ip);
            }
        }
    }
}

fn take_slot(counts: &Arc<Mutex<Counts>>, ip: IpAddr, limits: &Limits) -> Option<Slot> {
    let mut c = counts.lock().unwrap_or_else(PoisonError::into_inner);
    let mine = c.per_ip.get(&ip).copied().unwrap_or(0);
    if c.total >= limits.max_connections || mine >= limits.max_per_ip {
        return None;
    }
    c.total += 1;
    *c.per_ip.entry(ip).or_insert(0) += 1;
    Some(Slot {
        counts: counts.clone(),
        ip,
    })
}

/// Serve the LAN API on `listener` until `shutdown` turns true, then stop every
/// connection and wait for the blocking operations they started.
///
/// # Errors
/// A rustls error if the TLS configuration cannot be built.
pub async fn serve_api(
    listener: TcpListener,
    server: Arc<Server>,
    limits: Limits,
    mut shutdown: watch::Receiver<bool>,
) -> Result<(), rustls::Error> {
    let acceptor = TlsAcceptor::from(crate::tls::server_config(&server.keys)?);
    let router = api::router(server.clone());
    let counts = Arc::new(Mutex::new(Counts::default()));
    // Every open connection holds the server, and so does every blocking operation a
    // request started (it runs to the end even when its connection is stopped). On
    // shutdown the connections are stopped and awaited, then this waits until no
    // blocking operation is left running ([`Server::idle`]). So once this has returned,
    // and `serve_admin` too if it runs on the same server, nothing either listener
    // started holds a reference: what remains is the caller's own (tested by
    // `crates/hy-server/tests/shutdown.rs`). Other work the caller spawned with the
    // server, such as the executable's maintenance task, is the caller's to stop.
    let mut connections = tokio::task::JoinSet::new();
    loop {
        while connections.try_join_next().is_some() {}
        let accepted = tokio::select! {
            a = listener.accept() => a,
            _ = shutdown.changed() => {
                connections.shutdown().await;
                drop(router);
                server.idle().await;
                return Ok(());
            }
        };
        let Ok((tcp, peer)) = accepted else {
            tokio::time::sleep(Duration::from_millis(50)).await;
            continue;
        };
        let Some(slot) = take_slot(&counts, peer.ip(), &limits) else {
            logging::log(
                "connection_refused",
                &[("reason", V::Name("limit")), ("net", V::Net(peer.ip()))],
            );
            drop(tcp);
            continue;
        };
        let _ = tcp.set_nodelay(true);
        let acceptor = acceptor.clone();
        let router = router.clone();
        connections.spawn(async move {
            let _slot = slot;
            let Ok(Ok(tls)) =
                tokio::time::timeout(limits.handshake_timeout, acceptor.accept(tcp)).await
            else {
                return;
            };
            if tls.get_ref().1.alpn_protocol() != Some(ALPN) {
                return;
            }
            let svc = router.layer(axum::Extension(Peer(peer))).map_request(
                |r: hyper::Request<hyper::body::Incoming>| r.map(axum::body::Body::new),
            );
            let conn = hyper::server::conn::http1::Builder::new()
                .timer(TokioTimer::new())
                .header_read_timeout(limits.header_timeout)
                .keep_alive(true)
                .serve_connection(TokioIo::new(tls), TowerToHyperService::new(svc));
            let _ = tokio::time::timeout(limits.connection_cap, conn).await;
        });
    }
}

/// True if `addr` may host the admin page: loopback only (contract TH15).
#[must_use]
pub fn admin_address_ok(addr: &SocketAddr) -> bool {
    addr.ip().is_loopback()
}

/// Serve the admin page on `listener` (which must be bound to a loopback address) until
/// `shutdown` turns true, then stop and wait as [`serve_api`] does.
///
/// # Errors
/// `InvalidInput` if the listener is not on a loopback address.
pub async fn serve_admin(
    listener: TcpListener,
    server: Arc<Server>,
    mut shutdown: watch::Receiver<bool>,
) -> std::io::Result<()> {
    let local = listener.local_addr()?;
    if !admin_address_ok(&local) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "the admin page is served on a loopback address only",
        ));
    }
    let router = crate::admin::router(server.clone(), local.port());
    // As in `serve_api`: connections are stopped and awaited on shutdown, then the
    // blocking operations they started (a sign-in check) are waited for.
    let mut connections = tokio::task::JoinSet::new();
    loop {
        while connections.try_join_next().is_some() {}
        let accepted = tokio::select! {
            a = listener.accept() => a,
            _ = shutdown.changed() => {
                connections.shutdown().await;
                drop(router);
                server.idle().await;
                return Ok(());
            }
        };
        let Ok((tcp, _peer)) = accepted else {
            tokio::time::sleep(Duration::from_millis(50)).await;
            continue;
        };
        let router = router.clone();
        connections.spawn(async move {
            let svc = router.map_request(|r: hyper::Request<hyper::body::Incoming>| {
                r.map(axum::body::Body::new)
            });
            let conn = hyper::server::conn::http1::Builder::new()
                .timer(TokioTimer::new())
                .header_read_timeout(Duration::from_secs(10))
                .serve_connection(TokioIo::new(tcp), TowerToHyperService::new(svc));
            let _ = tokio::time::timeout(Duration::from_secs(120), conn).await;
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slots_are_limited_per_ip_and_in_total_and_released_on_drop() {
        let counts = Arc::new(Mutex::new(Counts::default()));
        let limits = Limits {
            max_connections: 3,
            max_per_ip: 2,
            ..Limits::default()
        };
        let a: IpAddr = "10.0.0.1".parse().unwrap();
        let b: IpAddr = "10.0.0.2".parse().unwrap();
        let s1 = take_slot(&counts, a, &limits).unwrap();
        let _s2 = take_slot(&counts, a, &limits).unwrap();
        assert!(take_slot(&counts, a, &limits).is_none(), "per IP");
        let _s3 = take_slot(&counts, b, &limits).unwrap();
        assert!(take_slot(&counts, b, &limits).is_none(), "total");
        drop(s1);
        assert!(take_slot(&counts, b, &limits).is_some());
    }

    #[test]
    fn the_admin_page_is_loopback_only() {
        assert!(admin_address_ok(&"127.0.0.1:8444".parse().unwrap()));
        assert!(admin_address_ok(&"[::1]:8444".parse().unwrap()));
        assert!(!admin_address_ok(&"0.0.0.0:8444".parse().unwrap()));
        assert!(!admin_address_ok(&"192.168.1.5:8444".parse().unwrap()));
    }
}
