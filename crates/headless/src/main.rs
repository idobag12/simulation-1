//! `embervale` — the headless CLI runner (SPEC §13).
//!
//! Subcommands:
//! - `run`             run a seed for N ticks, print `tick,hash` CSV, optionally save/load
//! - `verify`          run the same spec twice from scratch and compare hashes
//! - `save-load-check` compare an uninterrupted run against save → load → resume
//!
//! This is tooling (SPEC §3): errors print to stderr and set the exit code.

use std::path::PathBuf;
use std::process::ExitCode;

use core_types::Seed;
use headless::runner::{self, WorldSpec};

/// Exit code for a determinism mismatch (distinct from usage/runtime
/// errors so scripts can tell them apart).
const EXIT_MISMATCH: u8 = 1;
const EXIT_ERROR: u8 = 2;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match dispatch(&args) {
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

const USAGE: &str = "usage:
  embervale run --seed N --ticks N [--hash-interval N] [--fixture] [--entities N]
                [--save PATH] [--load PATH]
  embervale verify --seed N --ticks N [--hash-interval N] [--fixture] [--entities N]
  embervale save-load-check --seed N --ticks N [--resume-at N] [--fixture] [--entities N]

defaults: --hash-interval 10000, --entities 200, --resume-at ticks/2";

/// Returns Ok(true) on success, Ok(false) on determinism mismatch.
fn dispatch(args: &[String]) -> Result<bool, String> {
    let (command, rest) = args.split_first().ok_or("missing subcommand")?;
    let flags = Flags::parse(rest)?;
    match command.as_str() {
        "run" => cmd_run(&flags),
        "verify" => cmd_verify(&flags),
        "save-load-check" => cmd_save_load_check(&flags),
        other => Err(format!("unknown subcommand `{other}`")),
    }
}

/// Parsed command-line flags.
struct Flags {
    seed: Option<u64>,
    ticks: Option<u64>,
    hash_interval: u64,
    fixture: bool,
    entities: u32,
    resume_at: Option<u64>,
    save: Option<PathBuf>,
    load: Option<PathBuf>,
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
                other => return Err(format!("unknown flag `{other}`")),
            }
        }
        Ok(flags)
    }

    fn spec(&self) -> Result<WorldSpec, String> {
        let seed = Seed::new(self.seed.ok_or("--seed is required")?);
        Ok(if self.fixture {
            WorldSpec::fixture(seed, self.entities)
        } else {
            WorldSpec::empty(seed)
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
            let sim = persistence::load_from_file(path, runner::register_components)
                .map_err(|e| e.to_string())?;
            // The schedule is reconstructed from the flags, exactly like
            // component registrations (it is not part of saved state); the
            // seed comes from the save, so --seed is not consulted here.
            let schedule_spec = if flags.fixture {
                WorldSpec::fixture(sim.seed(), flags.entities)
            } else {
                WorldSpec::empty(sim.seed())
            };
            let schedule = runner::build_schedule(&schedule_spec);
            (sim, schedule)
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
    let mut resumed = persistence::load_from_bytes(&save, runner::register_components)
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
