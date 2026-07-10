//! The `embervale` CLI implementation (SPEC §13), as a library module so
//! the flag parser, command logic, and exit-code contract are unit-testable
//! (SPEC §16.5: no untested claims); `main.rs` is a thin shim over
//! [`main_with_args`].

use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;

use core_types::Seed;
use data_defs::DataDefs;

use crate::inspect;
use crate::runner::{self, WorldSpec};

/// Exit code for a determinism mismatch (distinct from usage/runtime errors
/// so scripts can tell them apart).
pub const EXIT_MISMATCH: u8 = 1;
/// Exit code for usage or runtime errors.
pub const EXIT_ERROR: u8 = 2;

/// Usage text printed on errors.
pub const USAGE: &str = "usage:
  embervale run --seed N --ticks N [--hash-interval N] [--fixture] [--entities N]
                [--citizens N] [--catch-up-days N] [--save PATH] [--load PATH]
                [--stats-csv PATH] [--data DIR]
  embervale verify --seed N --ticks N [--hash-interval N] [--fixture] [--entities N]
                [--citizens N] [--data DIR]
  embervale save-load-check --seed N --ticks N [--resume-at N] [--fixture] [--entities N]
                [--citizens N] [--data DIR]
  embervale inspect --load PATH --entity INDEX [--data DIR]
  embervale demography --load PATH [--data DIR]
  embervale economy --load PATH [--data DIR]
  embervale stories --load PATH [--entity INDEX] [--data DIR]

defaults: --hash-interval 10000, --entities 200, --citizens 0,
          --resume-at ticks/2, --data ./data
with --load, the save determines the world's composition; --fixture and
--citizens apply to fresh runs only";

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
        "inspect" => cmd_inspect(&flags),
        "demography" => cmd_demography(&flags),
        "economy" => cmd_economy(&flags),
        "stories" => cmd_stories(&flags),
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
    citizens: u32,
    resume_at: Option<u64>,
    catch_up_days: u64,
    save: Option<PathBuf>,
    load: Option<PathBuf>,
    stats_csv: Option<PathBuf>,
    entity: Option<u32>,
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
            citizens: 0,
            resume_at: None,
            catch_up_days: 0,
            save: None,
            load: None,
            stats_csv: None,
            entity: None,
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
                "--citizens" => flags.citizens = parse_num(value("--citizens")?, "--citizens")?,
                "--resume-at" => {
                    flags.resume_at = Some(parse_num(value("--resume-at")?, "--resume-at")?);
                }
                "--catch-up-days" => {
                    flags.catch_up_days = parse_num(value("--catch-up-days")?, "--catch-up-days")?;
                }
                "--entity" => flags.entity = Some(parse_num(value("--entity")?, "--entity")?),
                "--fixture" => flags.fixture = true,
                "--save" => flags.save = Some(PathBuf::from(value("--save")?)),
                "--load" => flags.load = Some(PathBuf::from(value("--load")?)),
                "--stats-csv" => flags.stats_csv = Some(PathBuf::from(value("--stats-csv")?)),
                "--data" => flags.data = PathBuf::from(value("--data")?),
                other => return Err(format!("unknown flag `{other}`")),
            }
        }
        Ok(flags)
    }

    /// Loads and validates the data definitions (SPEC §8: a data failure
    /// is a startup error with a precise message).
    fn defs(&self) -> Result<DataDefs, String> {
        data_defs::load(&self.data).map_err(|e| e.to_string())
    }

    fn spec(&self) -> Result<WorldSpec, String> {
        Ok(WorldSpec {
            seed: Seed::new(self.seed.ok_or("--seed is required")?),
            fixture: self.fixture,
            fixture_entities: self.entities,
            citizens: self.citizens,
            // Fresh builds derive their economy from the citizen count;
            // loads derive it from the save (ADR 0007 §8b).
            economy: false,
        })
    }

    fn ticks(&self) -> Result<u64, String> {
        self.ticks.ok_or("--ticks is required".into())
    }

    fn load_path(&self) -> Result<&PathBuf, String> {
        self.load.as_ref().ok_or("--load is required".into())
    }
}

fn parse_num<T: std::str::FromStr>(raw: &str, flag: &str) -> Result<T, String> {
    raw.parse()
        .map_err(|_| format!("{flag}: `{raw}` is not a valid number"))
}

/// Loads a save and validates its town against the loaded data
/// definitions (vector lengths vs. need/trait order — a mismatch is a
/// precise startup error, never a silent reinterpretation).
fn load_sim(flags: &Flags, defs: &DataDefs) -> Result<sim_time::Simulation, String> {
    let load_config = runner::load_config(defs).map_err(|e| e.to_string())?;
    let sim = persistence::load_from_file(flags.load_path()?, load_config, runner::register_world)
        .map_err(|e| e.to_string())?;
    sim_people::validate_town(sim.world(), &defs.people).map_err(|e| e.to_string())?;
    Ok(sim)
}

fn cmd_run(flags: &Flags) -> Result<bool, String> {
    let ticks = flags.ticks()?;
    let defs = flags.defs()?;
    let (mut sim, mut schedule) = match &flags.load {
        Some(_) => {
            // The save is authoritative for the world's composition (which
            // systems must run) exactly as it is for the seed: the schedule
            // is derived from the loaded world's content, never from
            // --fixture/--citizens (SPEC §9 load-and-continue equivalence).
            let sim = load_sim(flags, &defs)?;
            let derived = runner::derive_spec_from_world(sim.world()).map_err(|e| e.to_string())?;
            if flags.fixture || flags.citizens > 0 {
                eprintln!(
                    "note: --fixture/--citizens are ignored with --load; the save determines \
                     the world's composition ({} citizens, fixture: {})",
                    derived.citizens, derived.fixture
                );
            }
            let schedule = runner::build_schedule(&derived, &defs);
            (sim, schedule)
        }
        None => runner::build_simulation(&flags.spec()?, &defs).map_err(|e| e.to_string())?,
    };

    if flags.catch_up_days > 0 {
        // Fast-forward FIRST (SPEC §5): the coarse integrator covers the
        // elapsed span, then the per-tick run continues from there.
        runner::catch_up(&mut sim, &defs, flags.catch_up_days).map_err(|e| e.to_string())?;
        eprintln!(
            "caught up {} day(s); resuming at tick {}",
            flags.catch_up_days,
            sim.tick()
        );
    }
    let hashes = match &flags.stats_csv {
        Some(path) => run_with_stats(&mut sim, &mut schedule, ticks, flags.hash_interval, path)?,
        None => runner::run_with_hashes(&mut sim, &mut schedule, ticks, flags.hash_interval)
            .map_err(|e| e.to_string())?,
    };
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

/// Like `runner::run_with_hashes`, additionally appending one CSV row per
/// simulated day: `tick,population,deaths_total,employed,seeking,unmatched`
/// (SPEC §13 time-series; ADR 0005 §7, ADR 0008 §5). Death counting
/// observes `PersonDied` events from outside the simulation — an
/// observer, not a system; the labor columns read the clearing's
/// measured `LaborStats`.
fn run_with_stats(
    sim: &mut sim_time::Simulation,
    schedule: &mut core_ecs::Schedule,
    ticks: u64,
    hash_interval: u64,
    csv_path: &std::path::Path,
) -> Result<runner::HashSequence, String> {
    let mut csv = std::fs::File::create(csv_path).map_err(|e| e.to_string())?;
    writeln!(
        csv,
        "tick,population,deaths_total,employed,seeking,unmatched"
    )
    .map_err(|e| e.to_string())?;
    let mut deaths_total: u64 = 0;
    let mut hashes = Vec::new();
    for _ in 0..ticks {
        if hash_interval != 0 && sim.tick().raw().is_multiple_of(hash_interval) {
            hashes.push((sim.tick(), sim.state_hash().map_err(|e| e.to_string())?));
        }
        sim.step(schedule).map_err(|e| e.to_string())?;
        deaths_total += sim
            .world()
            .events::<sim_people::PersonDied>()
            .map_err(|e| e.to_string())?
            .len() as u64;
        if sim
            .tick()
            .raw()
            .is_multiple_of(core_types::calendar::TICKS_PER_DAY)
        {
            let population = inspect::population(sim.world()).map_err(|e| e.to_string())?;
            let labor = sim
                .world()
                .iter::<core_ecs::sim_interface::LaborStats>()
                .map_err(|e| e.to_string())?
                .next()
                .map(|(_, stats)| *stats)
                .unwrap_or_default();
            writeln!(
                csv,
                "{},{population},{deaths_total},{},{},{}",
                sim.tick(),
                labor.employed,
                labor.seeking,
                labor.unmatched
            )
            .map_err(|e| e.to_string())?;
        }
    }
    hashes.push((sim.tick(), sim.state_hash().map_err(|e| e.to_string())?));
    Ok(hashes)
}

fn cmd_verify(flags: &Flags) -> Result<bool, String> {
    let spec = flags.spec()?;
    let defs = flags.defs()?;
    let ticks = flags.ticks()?;
    let (a, b) = runner::verify_two_fresh_runs(&spec, &defs, ticks, flags.hash_interval)
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
    let defs = flags.defs()?;
    let ticks = flags.ticks()?;
    let resume_at = flags.resume_at.unwrap_or(ticks / 2);
    if resume_at > ticks {
        return Err("--resume-at must be <= --ticks".into());
    }

    // Uninterrupted run.
    let (mut solid, mut solid_schedule) =
        runner::build_simulation(&spec, &defs).map_err(|e| e.to_string())?;
    solid
        .run_ticks(&mut solid_schedule, ticks)
        .map_err(|e| e.to_string())?;
    let solid_hash = solid.state_hash().map_err(|e| e.to_string())?;

    // Save at `resume_at`, load, resume to the end.
    let (mut first, mut first_schedule) =
        runner::build_simulation(&spec, &defs).map_err(|e| e.to_string())?;
    first
        .run_ticks(&mut first_schedule, resume_at)
        .map_err(|e| e.to_string())?;
    let save = persistence::save_to_bytes(&first).map_err(|e| e.to_string())?;
    let load_config = runner::load_config(&defs).map_err(|e| e.to_string())?;
    let mut resumed = persistence::load_from_bytes(&save, load_config, runner::register_world)
        .map_err(|e| e.to_string())?;
    // Resume through the same derived-from-save path `run --load` uses,
    // so this check exercises the real resume schedule (ADR 0007 §8b) —
    // the two resume paths in this binary must never disagree.
    let derived = runner::derive_spec_from_world(resumed.world()).map_err(|e| e.to_string())?;
    let mut resumed_schedule = runner::build_schedule(&derived, &defs);
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

fn cmd_inspect(flags: &Flags) -> Result<bool, String> {
    let defs = flags.defs()?;
    let sim = load_sim(flags, &defs)?;
    let index = flags.entity.ok_or("--entity is required")?;
    print!("{}", inspect::inspect_entity(&sim, &defs, index)?);
    Ok(true)
}

fn cmd_demography(flags: &Flags) -> Result<bool, String> {
    let defs = flags.defs()?;
    let sim = load_sim(flags, &defs)?;
    print!("{}", inspect::demography(&sim, &defs)?);
    Ok(true)
}

fn cmd_stories(flags: &Flags) -> Result<bool, String> {
    let defs = data_defs::load(&flags.data).map_err(|e| e.to_string())?;
    let sim = load_sim(flags, &defs)?;
    let lines = debug_tools::stories(sim.world(), flags.entity).map_err(|e| e.to_string())?;
    if lines.is_empty() {
        println!("no stories in the retained window");
    }
    for line in lines {
        println!("{line}");
    }
    Ok(true)
}

fn cmd_economy(flags: &Flags) -> Result<bool, String> {
    let defs = flags.defs()?;
    let sim = load_sim(flags, &defs)?;
    print!("{}", inspect::economy(&sim, &defs)?);
    Ok(true)
}

#[cfg(test)]
#[path = "cli_tests.rs"]
mod tests;
