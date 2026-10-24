//! `hy-server service`: the server as a Windows service (slice V1-6b).
//!
//! The server installer registers `HealthierYouServer` to run
//! `hy-server.exe service --mirror <file> [--data-dir <dir>]` under the virtual account
//! `NT SERVICE\HealthierYouServer` (V1-5 contract 6.2), starting automatically (delayed)
//! and restarting after a failure. The service reports *running* only once both ports
//! are open, so the installer can tell a release that does not start and put the
//! previous one back. A stop or a Windows shutdown ends the server the same way Ctrl+C
//! does on a console.

use std::ffi::OsString;
use std::sync::Mutex;
use std::sync::mpsc;
use std::time::Duration;

use windows_service::service::{
    ServiceControl, ServiceControlAccept, ServiceExitCode, ServiceState, ServiceStatus, ServiceType,
};
use windows_service::service_control_handler::{self, ServiceControlHandlerResult};
use windows_service::{define_windows_service, service_dispatcher};

/// The service's name (and its virtual account's name).
pub const SERVICE_NAME: &str = "HealthierYouServer";

/// The exit code the service reports when the server could not start or stopped with
/// an error; the reason is in `<data folder>\logs` when logging had started, and in
/// `service-error.txt` in the data folder otherwise.
const FAILED: u32 = 1;

define_windows_service!(ffi_service_main, service_main);

/// Hand this process to the service manager; returns when the service has stopped.
///
/// # Errors
/// When the process was not started by the service manager.
pub fn run() -> Result<(), String> {
    service_dispatcher::start(SERVICE_NAME, ffi_service_main)
        .map_err(|e| format!("hy-server service runs only when Windows starts it: {e}"))
}

fn service_main(_arguments: Vec<OsString>) {
    let (tx, rx) = mpsc::channel::<()>();
    let tx = Mutex::new(Some(tx));
    let handler = move |control| match control {
        ServiceControl::Stop | ServiceControl::Shutdown | ServiceControl::Preshutdown => {
            if let Some(tx) = tx.lock().ok().and_then(|mut t| t.take()) {
                let _ = tx.send(());
            }
            ServiceControlHandlerResult::NoError
        }
        ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
        _ => ServiceControlHandlerResult::NotImplemented,
    };
    let Ok(status) = service_control_handler::register(SERVICE_NAME, handler) else {
        return;
    };
    let set = |state: ServiceState, accepted: ServiceControlAccept, exit: u32, wait: u64| {
        let _ = status.set_service_status(ServiceStatus {
            service_type: ServiceType::OWN_PROCESS,
            current_state: state,
            controls_accepted: accepted,
            exit_code: if exit == 0 {
                ServiceExitCode::Win32(0)
            } else {
                ServiceExitCode::ServiceSpecific(exit)
            },
            checkpoint: 0,
            wait_hint: Duration::from_secs(wait),
            process_id: None,
        });
    };
    // Key generation (first start) and the control-log checks can take a while.
    set(
        ServiceState::StartPending,
        ServiceControlAccept::empty(),
        0,
        120,
    );
    let ready = || {
        set(
            ServiceState::Running,
            ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN,
            0,
            0,
        );
    };
    let result = crate::service_body(rx, &ready);
    set(
        ServiceState::StopPending,
        ServiceControlAccept::empty(),
        0,
        30,
    );
    match result {
        Ok(()) => set(ServiceState::Stopped, ServiceControlAccept::empty(), 0, 0),
        Err(e) => {
            record_failure(&e);
            set(
                ServiceState::Stopped,
                ServiceControlAccept::empty(),
                FAILED,
                0,
            );
        }
    }
}

/// Keep the reason a start failed where the administrator (and the installer) can read
/// it: the data folder, which only SYSTEM, Administrators and the service can open.
fn record_failure(error: &str) {
    let dir = std::env::args()
        .skip_while(|a| a != "--data-dir")
        .nth(1)
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var_os("ProgramData").map(|p| {
                std::path::PathBuf::from(p)
                    .join("HealthierYou")
                    .join("Server")
            })
        });
    if let Some(dir) = dir {
        let when = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ");
        let _ = std::fs::write(dir.join("service-error.txt"), format!("{when} {error}\r\n"));
    }
}
