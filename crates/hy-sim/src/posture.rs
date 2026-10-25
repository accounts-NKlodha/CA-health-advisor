//! `hy-sim posture [--sweep] [<report.json>]` (slice V2-3): the posture check's synthetic
//! desk sessions ([`hy_core::posture::desk`]) scored against the PRD V2 gate.
//!
//! Without `--sweep` it prints the quality report for the shipped (provisional)
//! thresholds, each session's raw counts, and exits non-zero if the synthetic gate fails.
//! With `--sweep` it also tries the drift and viewpoint grids and prints one line per
//! setting, so a threshold change can be argued from raw counts. Everything printed is a
//! count, a time or a threshold; every line says it is synthetic. Thresholds tuned this way
//! stay provisional until real desk sessions pass and the owner approves them.

use std::fmt::Write as _;
use std::path::Path;

use hy_core::posture::Thresholds;
use hy_core::posture::desk;
use hy_core::posture::quality::GateReport;

fn sweep_line(th: &Thresholds, g: &GateReport) -> String {
    let mut line = format!(
        "z_drift {:>3} z_end {:>3} end {:>2}s band {:>4} turn {:>4} match {:>5}/{:>5} | ",
        th.z_drift,
        th.z_end,
        th.end_after_s,
        th.span_band_min,
        th.turn_tolerance,
        th.match_shift,
        th.match_scale
    );
    let (mut det, mut staged, mut fc, mut fe, mut fs) = (0, 0, 0, 0, 0);
    let mut lat = Vec::new();
    for s in &g.setups {
        det += s.detected;
        staged += s.staged_drifts;
        fc += s.false_cues;
        fe += s.false_episodes;
        fs += s.false_suspect_s;
        lat.extend(s.latency_s.iter().copied());
    }
    lat.sort_unstable();
    let median = lat.get(lat.len() / 2).copied().unwrap_or_default();
    let _ = write!(
        line,
        "detected {det}/{staged}, false cues {fc}, false episodes {fe}, false suspension {fs} s, median latency {median} s, setups passing {}/{} => {}",
        g.setups.iter().filter(|s| s.pass).count(),
        g.setups.len(),
        if g.pass { "PASS" } else { "FAIL" }
    );
    line
}

/// Run the command; the exit code.
pub fn cmd(args: &[String]) -> std::process::ExitCode {
    let sweep = args.iter().any(|a| a == "--sweep");
    let report = args.iter().find(|a| !a.starts_with("--"));
    let (sessions, gate) = desk::all(Thresholds::PROVISIONAL);
    println!("{}", gate.text());
    println!("Sessions (SYNTHETIC):");
    for s in &sessions {
        print!("{}", s.text());
    }
    let mut lines = Vec::new();
    if sweep {
        println!("\nSweep (SYNTHETIC; drift values, then viewpoint values):");
        for th in desk::drift_grid().iter().chain(&desk::viewpoint_grid()) {
            let (_, g) = desk::all(*th);
            let (_, c) = desk::all_seeded(*th, desk::CHECK_SEEDS);
            let line = format!(
                "{}\n    check set: {}",
                sweep_line(th, &g),
                sweep_line(th, &c).split(" | ").nth(1).unwrap_or_default()
            );
            println!("{line}");
            lines.push(line);
        }
    }
    if let Some(path) = report {
        let json = serde_json::json!({
            "source": "synthetic",
            "gate": gate,
            "sessions": sessions,
            "sweep": lines,
        });
        if let Err(e) = std::fs::write(
            Path::new(path),
            serde_json::to_string_pretty(&json).unwrap_or_default(),
        ) {
            eprintln!("hy-sim: could not write the report: {e}");
            return std::process::ExitCode::FAILURE;
        }
    }
    if gate.pass {
        std::process::ExitCode::SUCCESS
    } else {
        std::process::ExitCode::FAILURE
    }
}
