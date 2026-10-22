//! HealthierYou load simulator (architecture 5.1: "3 real clients plus 10 simulated
//! clients, driven by a simulator built on the shared core crate").
//!
//! V1-0 skeleton only. It shows the shared core runs headless: it opens an in-memory store
//! on a manual clock. The simulated sync clients are slice V1-5.

use std::sync::Arc;

use hy_core::clock::{Clock, ManualClock};
use hy_core::store::Store;

fn run() -> hy_core::Result<String> {
    let clock = Arc::new(ManualClock::new(chrono::DateTime::UNIX_EPOCH));
    let store = Store::open_in_memory(clock.clone())?;
    Ok(format!(
        "hy-sim {}: in-memory store at schema v{}, zone {}, clock {}; simulated clients arrive in V1-5",
        env!("CARGO_PKG_VERSION"),
        store.migration_report().to,
        store.user_zone()?,
        clock.now().to_rfc3339()
    ))
}

fn main() -> std::process::ExitCode {
    match run() {
        Ok(line) => {
            println!("{line}");
            std::process::ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("hy-sim: {e}");
            std::process::ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn opens_a_store_headless() {
        let line = super::run().unwrap();
        let schema = format!("schema v{}", hy_core::migrations::latest_version());
        assert!(line.contains(&schema));
        assert!(line.contains("Asia/Kolkata"));
    }
}
