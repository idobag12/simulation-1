//! The `embervale` CLI implementation (SPEC §13), as a library module so
//! the flag parser, command logic, and exit-code contract are unit-testable
//! (SPEC §16.5: no untested claims); `main.rs` is a thin shim over
//! [`main_with_args`].

use std::path::PathBuf;
use std::process::ExitCode;

use core_types::Seed;

use crate::runner::{self, SimConfig, WorldSpec};

/// Exit code for a determinism mismatch (distinct from usage/runtime errors
/// so scripts can tell them apart).
pub const EXIT_MISMATCH: u8 = 1;
/// Exit code for usage or runtime errors.
pub const EXIT_ERROR: u8 = 2;

/// Usage text printed on errors.
pub const USAGE: &str = "usage:
  embervale run --seed N --ticks N [--hash-interval N] [--fixture] [--entities N]
                [--save PATH] [--load PATH] [--data DIR]
  embervale verify --seed N --ticks N [--hash-interval N] [--fixture] [--entities N] [--data DIR]
  embervale save-load-check --seed N --ticks N [--resume-at N] [--fixture] [--entities N] [--data DIR]

defaults: --hash-interval 10000, --entities 200, --resume-at ticks/2, --data ./data";

/// Full CLI entry point: dispatches and maps the outcome to the exit-code
/// contract — success ⇒ 0, determinism mismatch ⇒ [`EXIT_MISMATCH`],
/// usage/runtime error ⇒ [`EXIT_ERROR`] (with message + usage on stderr).
pub fn main_with_args(args: &[String]) -> ExitCode {
    match dispatch(args) {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::from(EXIT_MISMATCH),
        Err(message) => {
            eprintln!("error: {message}");
            eprintln!();
            eprintln!("{USAGE}");
            ExitCode::from(EXIT_ERROR)
        }
    }
}

/// Parses and executes a command line (without the binary name).
///
/// Invariant: `Ok(true)` = success, `Ok(false)` = a determinism check ran
/// and FAILED, `Err` = usage or runtime error. This tri-state is the CLI's
/// exit-code contract.
pub fn dispatch(args: &[String]) -> Result<bool, String> {
    let (command, rest) = args.split_first().ok_or("missing subcommand")?;
    let flags = Flags::parse(rest)?;
    match command.as_str() {
        "run" => cmd_run(&flags),
        "verify" => cmd_verify(&flags),
        "save-load-check" => cmd_save_load_check(&flags),
        other => Err(format!("unknown subcommand `{other}`")),
    }
}

/// Parsed command-line flags. Defaults follow [`USAGE`].
struct Flags {
    seed: Option<u64>,
    ticks: Option<u64>,
    hash_interval: u64,
    fixture: bool,
    entities: u32,
    resume_at: Option<u64>,
    save: Option<PathBuf>,
    load: Option<PathBuf>,
    data: PathBuf,
}

impl Flags {
    fn parse(args: &[String]) -> Result<Flags, String> {
        let mut flags = Flags {
            seed: None,
            ticks: None,
            hash_interval: 10_000,
            fixture: false,
            entities: 200,
            resume_at: None,
            save: None,
            load: None,
            data: PathBuf::from("data"),
        };
        let mut iter = args.iter();
        while let Some(flag) = iter.next() {
            let mut value = |name: &str| -> Result<&String, String> {
                iter.next().ok_or(format!("{name} requires a value"))
            };
            match flag.as_str() {
                "--seed" => flags.seed = Some(parse_num(value("--seed")?, "--seed")?),
                "--ticks" => flags.ticks = Some(parse_num(value("--ticks")?, "--ticks")?),
                "--hash-interval" => {
                    flags.hash_interval = parse_num(value("--hash-interval")?, "--hash-interval")?;
                }
                "--entities" => flags.entities = parse_num(value("--entities")?, "--entities")?,
                "--resume-at" => {
                    flags.resume_at = Some(parse_num(value("--resume-at")?, "--resume-at")?);
                }
                "--fixture" => flags.fixture = true,
                "--save" => flags.save = Some(PathBuf::from(value("--save")?)),
                "--load" => flags.load = Some(PathBuf::from(value("--load")?)),
                "--data" => flags.data = PathBuf::from(value("--data")?),
                other => return Err(format!("unknown flag `{other}`")),
            }
        }
        Ok(flags)
    }

    /// Loads and validates the data definitions (SPEC §8: a data failure
    /// is a startup error with a precise message).
    fn sim_config(&self) -> Result<SimConfig, String> {
        let defs = data_defs::load(&self.data).map_err(|e| e.to_string())?;
        Ok(SimConfig::from_data(&defs))
    }

    fn spec(&self) -> Result<WorldSpec, String> {
        let seed = Seed::new(self.seed.ok_or("--seed is required")?);
        let config = self.sim_config()?;
        Ok(if self.fixture {
            WorldSpec::fixture(seed, self.entities, config)
        } else {
            WorldSpec::empty(seed, config)
        })
    }

    fn ticks(&self) -> Result<u64, String> {
        self.ticks.ok_or("--ticks is required".into())
    }
}

fn parse_num<T: std::str::FromStr>(raw: &str, flag: &str) -> Result<T, String> {
    raw.parse()
        .map_err(|_| format!("{flag}: `{raw}` is not a valid number"))
}

fn cmd_run(flags: &Flags) -> Result<bool, String> {
    let ticks = flags.ticks()?;
    let (mut sim, mut schedule) = match &flags.load {
        Some(path) => {
            // The schedule, calendar, and log capacity are reconstructed
            // from flags + data files, exactly like registrations (they are
            // not part of saved state); the seed comes from the save, so
            // --seed is not consulted here.
            let config = flags.sim_config()?;
            let schedule_spec = if flags.fixture {
                WorldSpec::fixture(Seed::new(0), flags.entities, config)
            } else {
                WorldSpec::empty(Seed::new(0), config)
            };
            let load_config = schedule_spec.load_config().map_err(|e| e.to_string())?;
            let sim = persistence::load_from_file(path, load_config, runner::register_world)
                .map_err(|e| e.to_string())?;
            (sim, runner::build_schedule(&schedule_spec))
        }
        None => runner::build_simulation(&flags.spec()?).map_err(|e| e.to_string())?,
    };

    let hashes = runner::run_with_hashes(&mut sim, &mut schedule, ticks, flags.hash_interval)
        .map_err(|e| e.to_string())?;
    println!("tick,hash");
    for (tick, hash) in &hashes {
        println!("{tick},{hash}");
    }

    if let Some(path) = &flags.save {
        persistence::save_to_file(&sim, path).map_err(|e| e.to_string())?;
        eprintln!("saved to {}", path.display());
    }
    Ok(true)
}

fn cmd_verify(flags: &Flags) -> Result<bool, String> {
    let spec = flags.spec()?;
    let ticks = flags.ticks()?;
    let (a, b) = runner::verify_two_fresh_runs(&spec, ticks, flags.hash_interval)
        .map_err(|e| e.to_string())?;
    if a == b {
        println!(
            "PASS: two fresh runs of seed {} for {ticks} ticks are hash-identical ({} checkpoints)",
            spec.seed,
            a.len()
        );
        Ok(true)
    } else {
        let divergence = a
            .iter()
            .zip(&b)
            .find(|(x, y)| x != y)
            .map(|((tick, ha), (_, hb))| format!("first divergence at tick {tick}: {ha} vs {hb}"))
            .unwrap_or_else(|| "checkpoint counts differ".to_owned());
        println!("FAIL: {divergence}");
        Ok(false)
    }
}

fn cmd_save_load_check(flags: &Flags) -> Result<bool, String> {
    let spec = flags.spec()?;
    let ticks = flags.ticks()?;
    let resume_at = flags.resume_at.unwrap_or(ticks / 2);
    if resume_at > ticks {
        return Err("--resume-at must be <= --ticks".into());
    }

    // Uninterrupted run.
    let (mut solid, mut solid_schedule) =
        runner::build_simulation(&spec).map_err(|e| e.to_string())?;
    solid
        .run_ticks(&mut solid_schedule, ticks)
        .map_err(|e| e.to_string())?;
    let solid_hash = solid.state_hash().map_err(|e| e.to_string())?;

    // Save at `resume_at`, load, resume to the end.
    let (mut first, mut first_schedule) =
        runner::build_simulation(&spec).map_err(|e| e.to_string())?;
    first
        .run_ticks(&mut first_schedule, resume_at)
        .map_err(|e| e.to_string())?;
    let save = persistence::save_to_bytes(&first).map_err(|e| e.to_string())?;
    let load_config = spec.load_config().map_err(|e| e.to_string())?;
    let mut resumed = persistence::load_from_bytes(&save, load_config, runner::register_world)
        .map_err(|e| e.to_string())?;
    let mut resumed_schedule = runner::build_schedule(&spec);
    resumed
        .run_ticks(&mut resumed_schedule, ticks - resume_at)
        .map_err(|e| e.to_string())?;
    let resumed_hash = resumed.state_hash().map_err(|e| e.to_string())?;

    if solid_hash == resumed_hash {
        println!(
            "PASS: save at tick {resume_at} + resume to {ticks} matches uninterrupted run ({solid_hash})"
        );
        Ok(true)
    } else {
        println!("FAIL: uninterrupted {solid_hash} != save/load/resume {resumed_hash}");
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
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
        // A value-less flag at the end swallows "--data", then the final
        // path is left as a dangling flag — still a usage error.
        assert!(dispatch(&args(&["run", "--seed"])).is_err());
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
    fn verify_and_save_load_check_pass_on_a_deterministic_world() {
        assert_eq!(
            dispatch(&args(&[
                "verify",
                "--seed",
                "5",
                "--ticks",
                "2000",
                "--fixture",
                "--entities",
                "50",
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
                "2000",
                "--fixture",
                "--entities",
                "50",
            ])),
            Ok(true)
        );
    }

    #[test]
    fn run_save_then_load_resumes_identically() {
        let dir = std::env::temp_dir().join("embervale-cli-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("roundtrip.embersave");
        let path_str = path.to_str().unwrap();

        assert_eq!(
            dispatch(&args(&[
                "run",
                "--seed",
                "9",
                "--ticks",
                "1000",
                "--fixture",
                "--entities",
                "50",
                "--save",
                path_str,
            ])),
            Ok(true)
        );
        assert_eq!(
            dispatch(&args(&[
                "run",
                "--load",
                path_str,
                "--ticks",
                "1000",
                "--fixture",
                "--entities",
                "50",
            ])),
            Ok(true)
        );
        std::fs::remove_file(&path).ok();
    }
}
