//! CLI contract tests (split from `cli.rs` for the SPEC §3 module-size
//! rule).

use super::*;

/// The repo's real data directory, resolved from this crate's location
/// so tests pass regardless of the runner's working directory.
fn data_dir() -> String {
    concat!(env!("CARGO_MANIFEST_DIR"), "/../../data").to_owned()
}

fn args(list: &[&str]) -> Vec<String> {
    let mut v: Vec<String> = list.iter().map(|s| (*s).to_owned()).collect();
    v.push("--data".to_owned());
    v.push(data_dir());
    v
}

#[test]
fn missing_subcommand_and_unknown_flags_are_usage_errors() {
    assert!(dispatch(&[]).is_err());
    assert!(dispatch(&args(&["frobnicate"])).is_err());
    assert!(dispatch(&args(&["run", "--bogus"])).is_err());
    assert!(dispatch(&args(&["run", "--seed", "abc", "--ticks", "1"])).is_err());
    assert!(dispatch(&args(&["run", "--ticks", "1"])).is_err()); // seed required
    assert!(dispatch(&args(&["verify", "--seed", "1"])).is_err()); // ticks required
    assert!(dispatch(&args(&["run", "--seed"])).is_err()); // dangling value
    assert!(dispatch(&args(&["inspect", "--entity", "0"])).is_err()); // load required
}

#[test]
fn missing_data_directory_is_a_precise_startup_error() {
    let result = dispatch(&[
        "verify".into(),
        "--seed".into(),
        "1".into(),
        "--ticks".into(),
        "10".into(),
        "--data".into(),
        "/nonexistent/data".into(),
    ]);
    match result {
        Err(message) => assert!(message.contains("calendar.ron"), "{message}"),
        other => panic!("expected data error, got {other:?}"),
    }
}

#[test]
fn resume_at_beyond_ticks_is_a_usage_error() {
    let result = dispatch(&args(&[
        "save-load-check",
        "--seed",
        "1",
        "--ticks",
        "10",
        "--resume-at",
        "11",
    ]));
    assert!(result.is_err());
}

#[test]
fn verify_and_save_load_check_pass_with_citizens_and_fixture() {
    assert_eq!(
        dispatch(&args(&[
            "verify",
            "--seed",
            "5",
            "--ticks",
            "3000",
            "--fixture",
            "--entities",
            "50",
            "--citizens",
            "120",
            "--hash-interval",
            "500",
        ])),
        Ok(true)
    );
    assert_eq!(
        dispatch(&args(&[
            "save-load-check",
            "--seed",
            "5",
            "--ticks",
            "3000",
            "--citizens",
            "120",
        ])),
        Ok(true)
    );
}

#[test]
fn run_save_inspect_demography_round_trip() {
    let dir = std::env::temp_dir().join("embervale-cli-test");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("town.embersave");
    let path_str = path.to_str().unwrap();
    let csv = dir.join("stats.csv");
    let csv_str = csv.to_str().unwrap();

    assert_eq!(
        dispatch(&args(&[
            "run",
            "--seed",
            "9",
            "--ticks",
            "3000",
            "--citizens",
            "150",
            "--save",
            path_str,
            "--stats-csv",
            csv_str,
        ])),
        Ok(true)
    );
    // Stats CSV has a header + at least two day rows (3000 ticks > 2 days).
    let stats = std::fs::read_to_string(&csv).unwrap();
    assert!(stats.starts_with("tick,population,deaths_total,employed,seeking,unmatched"));
    assert!(stats.lines().count() >= 3, "{stats}");

    assert_eq!(
        dispatch(&args(&["demography", "--load", path_str])),
        Ok(true)
    );
    // Inspect the first live entity (a household or citizen exists at
    // some low index; index 0 is the first genesis household).
    assert_eq!(
        dispatch(&args(&["inspect", "--load", path_str, "--entity", "0"])),
        Ok(true)
    );
    assert_eq!(
        dispatch(&args(&["inspect", "--load", path_str, "--entity", "1"])),
        Ok(true)
    );
    // The Phase 4 economy report renders from a save through the CLI.
    assert_eq!(dispatch(&args(&["economy", "--load", path_str])), Ok(true));

    std::fs::remove_file(&path).ok();
    std::fs::remove_file(&csv).ok();
}
