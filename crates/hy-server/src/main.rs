//! HealthierYou office LAN server (architecture 5).
//!
//! V1-0 skeleton only: it proves the workspace builds the server binary against
//! `hy-core`. The service, API, pairing (5.2), control log and recovery hold (5.3) are
//! slice V1-5, which starts only after the pairing security gate is signed off.

fn banner() -> String {
    format!(
        "hy-server {} (hy-core {}, schema v{}): skeleton only; the server is built in slice V1-5",
        env!("CARGO_PKG_VERSION"),
        hy_core::VERSION,
        hy_core::migrations::latest_version()
    )
}

fn main() {
    println!("{}", banner());
}

#[cfg(test)]
mod tests {
    #[test]
    fn banner_names_the_slice() {
        let b = super::banner();
        assert!(b.contains("V1-5"));
        let schema = format!("schema v{}", hy_core::migrations::latest_version());
        assert!(b.contains(&schema));
    }
}
