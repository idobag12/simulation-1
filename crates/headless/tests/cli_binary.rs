//! Binary-level tests of the `embervale` executable: the exit-code contract
//! and stdout shape, exercised through a real process (SPEC §16.5 — the CLI
//! is a Phase 0 deliverable, so its observable behavior is tested, not
//! claimed).
//!
//! Exit 1 (determinism mismatch) cannot be triggered through a real
//! invocation precisely because the simulation is deterministic; its
//! mapping from `Ok(false)` is covered by `cli::dispatch`'s tri-state
//! contract tests in the library.

use std::process::Command;

/// The repo's real data directory, resolved from this crate's location.
const DATA: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../data");

fn embervale() -> Command {
    Command::new(env!("CARGO_BIN_EXE_embervale"))
}

#[test]
fn verify_passes_with_exit_zero_and_prints_pass() {
    let out = embervale()
        .args([
            "verify",
            "--seed",
            "3",
            "--ticks",
            "1000",
            "--fixture",
            "--entities",
            "50",
            "--hash-interval",
            "250",
            "--data",
            DATA,
        ])
        .output()
        .expect("failed to spawn embervale");
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8(out.stdout).expect("utf8");
    assert!(stdout.starts_with("PASS:"), "stdout was: {stdout}");
}

#[test]
fn usage_errors_exit_two_with_usage_on_stderr() {
    for bad in [
        vec![],
        vec!["frobnicate"],
        vec!["run", "--ticks", "10", "--data", DATA], // --seed missing
        vec!["run", "--seed", "nope", "--ticks", "10", "--data", DATA],
    ] {
        let out = embervale().args(&bad).output().expect("spawn failed");
        assert_eq!(out.status.code(), Some(2), "args: {bad:?}");
        let stderr = String::from_utf8(out.stderr).expect("utf8");
        assert!(stderr.contains("usage:"), "stderr for {bad:?}: {stderr}");
    }
}

#[test]
fn run_prints_csv_and_save_load_round_trips_through_files() {
    let dir = std::env::temp_dir().join("embervale-bin-test");
    std::fs::create_dir_all(&dir).expect("mkdir");
    let save = dir.join("bin_roundtrip.embersave");

    // Solid run to tick 2000.
    let solid = embervale()
        .args([
            "run",
            "--seed",
            "21",
            "--ticks",
            "2000",
            "--fixture",
            "--entities",
            "50",
            "--hash-interval",
            "1000",
            "--data",
            DATA,
        ])
        .output()
        .expect("spawn failed");
    assert_eq!(solid.status.code(), Some(0));
    let solid_stdout = String::from_utf8(solid.stdout).expect("utf8");
    let mut lines = solid_stdout.lines();
    assert_eq!(lines.next(), Some("tick,hash"));
    let solid_final = solid_stdout.lines().last().expect("no csv rows").to_owned();
    assert!(solid_final.starts_with("2000,"));

    // Save at tick 1000, then resume from the file for 1000 more.
    let first = embervale()
        .args([
            "run",
            "--seed",
            "21",
            "--ticks",
            "1000",
            "--fixture",
            "--entities",
            "50",
            "--save",
            save.to_str().expect("path utf8"),
            "--data",
            DATA,
        ])
        .output()
        .expect("spawn failed");
    assert_eq!(first.status.code(), Some(0));

    let resumed = embervale()
        .args([
            "run",
            "--load",
            save.to_str().expect("path utf8"),
            "--ticks",
            "1000",
            "--fixture",
            "--data",
            DATA,
        ])
        .output()
        .expect("spawn failed");
    assert_eq!(resumed.status.code(), Some(0));
    let resumed_stdout = String::from_utf8(resumed.stdout).expect("utf8");
    let resumed_final = resumed_stdout.lines().last().expect("no csv rows");

    assert_eq!(
        solid_final, resumed_final,
        "file-based resume diverged from the uninterrupted run"
    );
    std::fs::remove_file(&save).ok();
}

#[test]
fn loading_a_missing_file_reports_io_error_and_exits_two() {
    let out = embervale()
        .args([
            "run",
            "--load",
            "/nonexistent/nowhere.embersave",
            "--ticks",
            "1",
            "--data",
            DATA,
        ])
        .output()
        .expect("spawn failed");
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8(out.stderr).expect("utf8");
    assert!(
        stderr.contains("io on"),
        "missing file must surface as an io error, got: {stderr}"
    );
}
