//! `embervale-viewer` (Phase 10, ADR 0013 §3): one sim thread, one UI
//! thread, snapshots over a channel. Usage:
//! `embervale-viewer --data data [--load save.embersave | --seed N --citizens N]`.

#![forbid(unsafe_code)]

use std::sync::mpsc;

use core_types::Seed;
use core_types::calendar::TICKS_PER_DAY;
use debug_tools::metrics::MetricsRegistry;
use headless::runner::{self, WorldSpec};
use viewer::app::{SimCommand, SimMessage, ViewerApp};
use viewer::snapshot::Snapshot;

fn main() {
    if let Err(message) = run() {
        eprintln!("error: {message}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let mut data_root = String::from("data");
    let mut load: Option<String> = None;
    let mut seed: Option<u64> = None;
    let mut citizens: Option<u32> = None;
    let mut arguments = std::env::args().skip(1);
    while let Some(argument) = arguments.next() {
        let mut value = |name: &str| {
            arguments
                .next()
                .ok_or_else(|| format!("{name} needs a value"))
        };
        match argument.as_str() {
            "--data" => data_root = value("--data")?,
            "--load" => load = Some(value("--load")?),
            "--seed" => {
                seed = Some(
                    value("--seed")?
                        .parse()
                        .map_err(|error| format!("--seed: {error}"))?,
                );
            }
            "--citizens" => {
                citizens = Some(
                    value("--citizens")?
                        .parse()
                        .map_err(|error| format!("--citizens: {error}"))?,
                );
            }
            other => return Err(format!("unknown argument `{other}`")),
        }
    }

    let defs = data_defs::load(std::path::Path::new(&data_root))
        .map_err(|error| format!("data: {error}"))?;
    let (mut sim, mut schedule) = match &load {
        Some(path) => {
            if seed.is_some() || citizens.is_some() {
                eprintln!("note: --seed/--citizens are ignored with --load (the save decides)");
            }
            let sim = persistence::load_from_file(
                std::path::Path::new(path),
                runner::load_config(&defs).map_err(|error| format!("config: {error}"))?,
                runner::register_world,
            )
            .map_err(|error| format!("load: {error}"))?;
            // The same guard the CLI runs: a save/data mismatch is a
            // precise startup error, never a silent reinterpretation.
            sim_people::validate_town(sim.world(), &defs.people)
                .map_err(|error| format!("load: {error}"))?;
            let spec =
                runner::derive_spec_from_world(sim.world()).map_err(|error| error.to_string())?;
            let schedule = runner::build_schedule(&spec, &defs);
            (sim, schedule)
        }
        None => {
            let spec = WorldSpec::town(Seed::new(seed.unwrap_or(1)), citizens.unwrap_or(250));
            runner::build_simulation(&spec, &defs).map_err(|error| error.to_string())?
        }
    };
    schedule.enable_timing();

    let (command_sender, command_receiver) = mpsc::channel::<SimCommand>();
    let (message_sender, message_receiver) = mpsc::channel::<SimMessage>();

    // The sim thread (ADR 0013 §3): each command says HOW MANY ticks to
    // step; one snapshot answers each command, so the UI's frame rate
    // paces the sim while running. Errors are messages, never silence.
    std::thread::spawn(move || {
        let mut metrics = MetricsRegistry::new();
        if let Err(error) = metrics.sample(sim.world(), sim.tick().raw() / TICKS_PER_DAY) {
            let _ = message_sender.send(SimMessage::Halted(format!("metrics: {error}")));
            return;
        }
        let send_snapshot = |sim: &sim_time::Simulation,
                             schedule: &core_ecs::Schedule,
                             metrics: &MetricsRegistry|
         -> Result<(), String> {
            let snapshot = Snapshot::capture(
                sim.world(),
                sim.tick().raw(),
                &defs,
                schedule.last_timings(),
                metrics,
            )
            .map_err(|error| format!("snapshot: {error}"))?;
            let _ = message_sender.send(SimMessage::Snapshot(Box::new(snapshot)));
            Ok(())
        };
        if let Err(reason) = send_snapshot(&sim, &schedule, &metrics) {
            let _ = message_sender.send(SimMessage::Halted(reason));
            return;
        }
        loop {
            // Block for the next command, then collapse any backlog —
            // the LATEST command wins, so a pause is never starved
            // behind stale run commands.
            let mut command = match command_receiver.recv() {
                Ok(command) => command,
                Err(_) => return,
            };
            while let Ok(next) = command_receiver.try_recv() {
                command = next;
            }
            let ticks = match command {
                SimCommand::Pause => 0,
                SimCommand::Step(ticks) | SimCommand::Run(ticks) => ticks,
            };
            if ticks > 0
                && let Err(reason) = step_sampling(&mut sim, &mut schedule, &mut metrics, ticks)
            {
                let _ = message_sender.send(SimMessage::Halted(reason));
                return;
            }
            if let Err(reason) = send_snapshot(&sim, &schedule, &metrics) {
                let _ = message_sender.send(SimMessage::Halted(reason));
                return;
            }
        }
    });

    eframe::run_native(
        "Embervale",
        eframe::NativeOptions::default(),
        Box::new(move |_| Ok(Box::new(ViewerApp::new(command_sender, message_receiver)))),
    )
    .map_err(|error| format!("viewer: {error}"))
}

/// Steps `ticks`, pausing at every day boundary to sample the metrics
/// registry — a multi-day batch records every day, not just the last.
fn step_sampling(
    sim: &mut sim_time::Simulation,
    schedule: &mut core_ecs::Schedule,
    metrics: &mut MetricsRegistry,
    ticks: u64,
) -> Result<(), String> {
    let mut remaining = ticks;
    while remaining > 0 {
        let into_day = sim.tick().raw() % TICKS_PER_DAY;
        let chunk = remaining.min(TICKS_PER_DAY - into_day);
        sim.run_ticks(schedule, chunk)
            .map_err(|error| format!("simulation halted: {error}"))?;
        remaining -= chunk;
        let tick = sim.tick().raw();
        if tick.is_multiple_of(TICKS_PER_DAY) {
            metrics
                .sample(sim.world(), tick / TICKS_PER_DAY)
                .map_err(|error| format!("metrics: {error}"))?;
        }
    }
    Ok(())
}
