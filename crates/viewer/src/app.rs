//! The paint layer (Phase 10, ADR 0013 §§1, 3–4): draws the pane
//! builders' data and forwards clicks. Every decision worth testing
//! lives in `snapshot`/`panes`/`events`; this file only maps classes
//! to colors, samples to pixels, and clicks to focus changes.

use std::sync::mpsc::{Receiver, Sender};

use crate::panes::{self, Overlay};
use crate::snapshot::Snapshot;

/// Commands the UI sends the sim thread (ADR 0013 §3): only HOW MANY
/// ticks to step — never what happens in them.
#[derive(Debug, Clone, Copy)]
pub enum SimCommand {
    /// Stop stepping (also collapses any queued run commands).
    Pause,
    /// Step exactly this many ticks, then pause.
    Step(u64),
    /// Step this many ticks for THIS command — the UI sends one per
    /// frame while running, so the frame rate paces the sim.
    Run(u64),
}

/// What the sim thread sends back: a snapshot per step batch, or the
/// reason it stopped forever (a halted audit is a message, not a
/// silent freeze).
#[derive(Debug)]
pub enum SimMessage {
    /// A fresh snapshot after a step batch.
    Snapshot(Box<Snapshot>),
    /// The sim halted; no further snapshots will come.
    Halted(String),
}

/// What the inspector is looking at — citizens AND locations (firms,
/// homes, venues) are inspectable (SPEC §13).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Focus {
    Citizen(u32),
    Location(u32),
}

/// The eframe application: one snapshot at a time, panes on the left,
/// the map center, the inspector right.
pub struct ViewerApp {
    commands: Sender<SimCommand>,
    messages: Receiver<SimMessage>,
    latest: Option<Snapshot>,
    halted: Option<String>,
    search: String,
    focus: Option<Focus>,
    follow: bool,
    overlay: Overlay,
    event_kind_filter: String,
    filter_ticks: bool,
    filter_from: u64,
    filter_to: u64,
    running: bool,
    speed: u64,
}

/// The fixed five-class palette every overlay maps to (class 0 = cool,
/// class 4 = hot).
fn class_color(class: u8) -> egui::Color32 {
    match class {
        0 => egui::Color32::from_rgb(0x35, 0x6f, 0xc5),
        1 => egui::Color32::from_rgb(0x3f, 0xa5, 0x8f),
        2 => egui::Color32::from_rgb(0xd0, 0xc0, 0x4a),
        3 => egui::Color32::from_rgb(0xe0, 0x8a, 0x33),
        _ => egui::Color32::from_rgb(0xd9, 0x3a, 0x3a),
    }
}

impl ViewerApp {
    /// Wires the app to the sim thread's channels.
    pub fn new(commands: Sender<SimCommand>, messages: Receiver<SimMessage>) -> Self {
        ViewerApp {
            commands,
            messages,
            latest: None,
            halted: None,
            search: String::new(),
            focus: None,
            follow: false,
            overlay: Overlay::Wealth,
            event_kind_filter: String::new(),
            filter_ticks: false,
            filter_from: 0,
            filter_to: 0,
            running: false,
            speed: 60,
        }
    }

    fn drain_messages(&mut self) {
        while let Ok(message) = self.messages.try_recv() {
            match message {
                SimMessage::Snapshot(snapshot) => self.latest = Some(*snapshot),
                SimMessage::Halted(reason) => {
                    self.halted = Some(reason);
                    self.running = false;
                }
            }
        }
    }

    fn time_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if let Some(snapshot) = &self.latest {
                ui.label(format!("day {} · tick {}", snapshot.day, snapshot.tick));
            }
            if ui.button("⏸ pause").clicked() {
                self.running = false;
                let _ = self.commands.send(SimCommand::Pause);
            }
            if ui.button("step tick").clicked() {
                let _ = self.commands.send(SimCommand::Step(1));
            }
            if ui.button("step day").clicked() {
                let _ = self.commands.send(SimCommand::Step(1440));
            }
            if ui.button("▶ run").clicked() && self.halted.is_none() {
                self.running = true;
            }
            ui.add(egui::Slider::new(&mut self.speed, 1..=1440).text("ticks/frame"));
            if let Some(reason) = &self.halted {
                ui.colored_label(
                    egui::Color32::from_rgb(0xd9, 0x3a, 0x3a),
                    format!("simulation halted: {reason}"),
                );
            }
        });
    }

    fn left_panel(&mut self, ui: &mut egui::Ui, snapshot: &Snapshot) {
        ui.heading("overlays");
        ui.radio_value(&mut self.overlay, Overlay::Wealth, "wealth");
        for (index, need) in snapshot.need_names.iter().enumerate() {
            ui.radio_value(
                &mut self.overlay,
                Overlay::Need(index as u32),
                format!("need: {need}"),
            );
        }
        ui.radio_value(&mut self.overlay, Overlay::Tier, "LOD tier");
        ui.radio_value(&mut self.overlay, Overlay::Price, "posted prices");

        ui.separator();
        ui.heading("dashboard");
        let (income_tax, sales_tax) = snapshot.treasury_receipts_mills;
        ui.label(format!(
            "treasury receipts: {income_tax} income + {sales_tax} sales (mills)"
        ));
        for (name, samples) in &snapshot.metrics {
            let last = samples.last().map(|(_, value)| *value).unwrap_or(0);
            ui.label(format!("{name}: {last}"));
            let points = panes::normalize_series(samples);
            if points.len() >= 2 {
                let (response, painter) = ui.allocate_painter(
                    egui::vec2(ui.available_width().min(180.0), 24.0),
                    egui::Sense::hover(),
                );
                let rect = response.rect.shrink(1.0);
                let to_screen = |(x, y): (f32, f32)| {
                    egui::pos2(
                        rect.left() + x * rect.width(),
                        rect.bottom() - y * rect.height(),
                    )
                };
                let line: Vec<egui::Pos2> = points.into_iter().map(to_screen).collect();
                painter.add(egui::Shape::line(
                    line,
                    egui::Stroke::new(1.0, egui::Color32::from_rgb(0x3f, 0xa5, 0x8f)),
                ));
            }
        }

        ui.separator();
        ui.heading("commute flows");
        for (home, work, count) in panes::commute_flows(snapshot) {
            let home = snapshot.districts.get(home as usize).cloned();
            let work = snapshot.districts.get(work as usize).cloned();
            if let (Some(home), Some(work)) = (home, work) {
                ui.label(format!("{home} → {work}: {count}"));
            }
        }

        ui.separator();
        ui.heading("timings (µs, last tick)");
        for (system, micros) in &snapshot.timings {
            ui.label(format!("{system}: {micros}"));
        }
    }

    fn inspector(&mut self, ui: &mut egui::Ui, snapshot: &Snapshot) {
        ui.heading("inspector");
        ui.horizontal(|ui| {
            ui.text_edit_singleline(&mut self.search);
            if ui.button("find").clicked()
                && let Some(first) = panes::search(snapshot, &self.search).first()
            {
                self.focus = Some(Focus::Citizen(*first));
            }
        });
        ui.checkbox(&mut self.follow, "follow");
        match self.focus {
            Some(Focus::Citizen(index)) => self.citizen_inspector(ui, snapshot, index),
            Some(Focus::Location(index)) => self.location_inspector(ui, snapshot, index),
            None => {
                ui.label("search a citizen or click the map");
            }
        }
    }

    fn citizen_inspector(&mut self, ui: &mut egui::Ui, snapshot: &Snapshot, index: u32) {
        let Some(view) = panes::inspect(snapshot, index) else {
            ui.label(format!("citizen #{index} is no longer in the town"));
            ui.label("(their events remain in the browser below)");
            return;
        };
        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.label(format!("#{} {}", view.row.index, view.row.name));
            ui.label(format!(
                "cash {} · deposit {} · tier {:?}",
                view.row.cash_mills, view.row.deposit_mills, view.row.tier
            ));
            if let Some(home) = view.row.home {
                match view.row.rent_mills {
                    Some(rent) => ui.label(format!("home #{home} (rents at {rent} mills/day)")),
                    None => ui.label(format!("home #{home}")),
                };
            }
            if let Some(employer) = view.row.employer {
                ui.label(format!("works at #{employer}"));
            }
            if let Some(action) = &view.row.action {
                ui.label(format!("doing: {action}"));
            }
            if let Some((start, end)) = view.row.sleep_window {
                ui.label(format!("sleeps {start}–{end} (minutes of day)"));
            }

            ui.separator();
            ui.label("needs:");
            for (need, level) in snapshot.need_names.iter().zip(&view.row.needs) {
                ui.label(format!("  {need}: {level}/1000000"));
            }
            if !view.row.skills.is_empty() {
                ui.label(format!("skills (per-mille): {:?}", view.row.skills));
            }

            if !view.row.bonds.is_empty() {
                ui.separator();
                ui.label("bonds:");
                for edge in panes::relationship_graph(snapshot, index) {
                    let place = edge
                        .other_at
                        .map(|at| format!(" — now at #{at}"))
                        .unwrap_or_default();
                    if ui
                        .link(format!(
                            "  {} #{} ({}, {}‰){place}",
                            edge.other_name, edge.other, edge.kind, edge.strength_per_mille
                        ))
                        .clicked()
                    {
                        self.focus = Some(Focus::Citizen(edge.other));
                    }
                }
            }

            if !view.row.believed.is_empty() {
                ui.separator();
                ui.label("believed prices:");
                for (shop, price) in &view.row.believed {
                    ui.label(format!("  shop #{shop}: {price} mills"));
                }
            }

            if let Some((tick, candidates)) = &view.row.last_decision {
                ui.separator();
                ui.label(format!("last decision (t{tick}):"));
                for (label, score) in candidates {
                    ui.label(format!("  {score:>10}µ  {label}"));
                }
            }

            ui.separator();
            ui.label("recent events:");
            for event in view.events.iter().rev() {
                ui.label(format!("t{} {}", event.tick, event.text));
            }
            ui.separator();
            for line in &view.stories {
                ui.label(line);
            }
        });
    }

    fn location_inspector(&mut self, ui: &mut egui::Ui, snapshot: &Snapshot, index: u32) {
        let Some(view) = panes::inspect_location(snapshot, index) else {
            ui.label(format!("location #{index} is not in this snapshot"));
            return;
        };
        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.label(format!("#{} {}", view.row.index, view.row.kind));
            if let Some(district) = view
                .row
                .district
                .and_then(|d| snapshot.districts.get(d as usize))
            {
                ui.label(format!("district: {district}"));
            }
            if let Some(price) = view.row.price_mills {
                ui.label(format!("posted price: {price} mills"));
            }
            if let Some(stock) = view.row.stock {
                ui.label(format!("stock: {stock}"));
            }
            ui.label(format!(
                "{} employees · {} present",
                view.employees.len(),
                view.present.len()
            ));
            for employee in &view.employees {
                if ui.link(format!("  employee #{employee}")).clicked() {
                    self.focus = Some(Focus::Citizen(*employee));
                }
            }
            ui.separator();
            ui.label("recent events:");
            for event in view.events.iter().rev() {
                ui.label(format!("t{} {}", event.tick, event.text));
            }
        });
    }

    fn event_browser(&mut self, ui: &mut egui::Ui, snapshot: &Snapshot) {
        ui.horizontal(|ui| {
            ui.label("event filter:");
            ui.text_edit_singleline(&mut self.event_kind_filter);
            ui.checkbox(&mut self.filter_ticks, "tick range");
            if self.filter_ticks {
                ui.add(egui::DragValue::new(&mut self.filter_from).prefix("from t"));
                ui.add(egui::DragValue::new(&mut self.filter_to).prefix("to t"));
            }
        });
        let kind = (!self.event_kind_filter.is_empty()).then_some(self.event_kind_filter.as_str());
        let entity = match self.focus {
            Some(Focus::Citizen(index)) | Some(Focus::Location(index)) => Some(index),
            None => None,
        };
        let ticks = self
            .filter_ticks
            .then_some((self.filter_from, self.filter_to));
        egui::ScrollArea::vertical()
            .max_height(120.0)
            .show(ui, |ui| {
                for event in crate::events::filter(&snapshot.events, entity, kind, ticks)
                    .iter()
                    .rev()
                    .take(200)
                {
                    ui.label(format!("t{} [{}] {}", event.tick, event.kind, event.text));
                }
            });
    }

    fn map(&mut self, ui: &mut egui::Ui, snapshot: &Snapshot) {
        let followed_at = match (self.follow, self.focus) {
            (true, Some(Focus::Citizen(index))) => snapshot
                .citizens
                .iter()
                .find(|row| row.index == index)
                .and_then(|row| row.at),
            _ => None,
        };
        egui::ScrollArea::both().show(ui, |ui| {
            ui.columns(snapshot.districts.len() + 1, |columns| {
                for (column, (district, locations)) in
                    panes::map_columns(snapshot).into_iter().enumerate()
                {
                    let ui = &mut columns[column];
                    ui.heading(district);
                    for location in locations {
                        self.map_location(ui, snapshot, location, followed_at);
                    }
                }
            });
        });
    }

    fn map_location(
        &mut self,
        ui: &mut egui::Ui,
        snapshot: &Snapshot,
        location: u32,
        followed_at: Option<u32>,
    ) {
        let row = snapshot
            .locations
            .iter()
            .find(|candidate| candidate.index == location);
        let label = row
            .map(|row| format!("#{} {}", row.index, row.kind))
            .unwrap_or_default();
        let present = panes::present_at(snapshot, location);
        let marker = if followed_at == Some(location) {
            "▶ "
        } else {
            ""
        };
        let text = format!("{marker}{label} ({} here)", present.len());
        let button = match row
            .and_then(|row| panes::location_overlay_class(snapshot, self.overlay, row))
        {
            Some(class) => egui::Button::new(text).fill(class_color(class).gamma_multiply(0.35)),
            None => egui::Button::new(text),
        };
        if ui.add(button).clicked() {
            self.focus = Some(Focus::Location(location));
        }
        // The presence dots, colored by the active overlay's class.
        ui.horizontal_wrapped(|ui| {
            for citizen in present.iter().take(24) {
                if let Some(citizen_row) = snapshot
                    .citizens
                    .iter()
                    .find(|candidate| candidate.index == *citizen)
                {
                    let class = panes::overlay_class(snapshot, self.overlay, citizen_row);
                    let dot = egui::RichText::new("●").color(class_color(class));
                    if ui
                        .add(egui::Label::new(dot).sense(egui::Sense::click()))
                        .on_hover_text(&citizen_row.name)
                        .clicked()
                    {
                        self.focus = Some(Focus::Citizen(*citizen));
                    }
                }
            }
            if present.len() > 24 {
                ui.label(format!("+{}", present.len() - 24));
            }
        });
    }
}

impl eframe::App for ViewerApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.drain_messages();

        egui::TopBottomPanel::top("time").show(ctx, |ui| self.time_bar(ui));

        let Some(snapshot) = self.latest.clone() else {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.label("waiting for the first snapshot…");
            });
            return;
        };

        egui::SidePanel::left("panes").show(ctx, |ui| self.left_panel(ui, &snapshot));
        egui::SidePanel::right("inspector").show(ctx, |ui| self.inspector(ui, &snapshot));
        egui::TopBottomPanel::bottom("events").show(ctx, |ui| self.event_browser(ui, &snapshot));
        egui::CentralPanel::default().show(ctx, |ui| self.map(ui, &snapshot));

        // One Run command per painted frame: the frame rate paces the
        // sim (ADR 0013 §3 — "N ticks per frame" is literal).
        if self.running && self.halted.is_none() {
            let _ = self.commands.send(SimCommand::Run(self.speed));
            ctx.request_repaint();
        }
    }
}
