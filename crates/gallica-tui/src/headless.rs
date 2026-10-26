//! The non-interactive commands: drive the core and print to the terminal.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use gallica_core::engine::{EventReceiver, channel};
use gallica_core::log::LogLayer;
use gallica_core::{
    Action, App, Command, Engine, Event, ItemStatus, LogLevel, Phase, Resolved, TargetStatus,
    format,
};
use indicatif::{MultiProgress, ProgressBar, ProgressDrawTarget, ProgressStyle};

use crate::cli::{Cli, load_settings};
use crate::{EXIT_BLOCKED, EXIT_PARTIAL, EXIT_USAGE};

/// What the command should do once the target is known.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Print the resolved target.
    Info,
    /// Print the plan and its estimate.
    Plan,
    /// Download.
    Download,
}

/// The log output of the headless commands: lines on stderr, above the
/// progress bars while they are shown.
#[derive(Clone, Default)]
pub struct Printer {
    bars: Arc<Mutex<Option<MultiProgress>>>,
}

impl Printer {
    /// The log output.
    pub fn layer(&self) -> LogLayer {
        let printer = self.clone();
        LogLayer::new(move |entry| printer.println(&entry.line()))
    }

    fn println(&self, line: &str) {
        let bars = self.bars.lock().ok().and_then(|b| b.clone());
        match bars {
            Some(multi) => {
                let _ = multi.println(line);
            }
            None => eprintln!("{line}"),
        }
    }

    fn set_bars(&self, multi: Option<MultiProgress>) {
        if let Ok(mut bars) = self.bars.lock() {
            *bars = multi;
        }
    }
}

/// Runs one headless command and returns the exit code.
pub async fn run(cli: &Cli, printer: &Printer, mode: Mode, target: &str) -> u8 {
    let settings = match load_settings(cli) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: {e}");
            return EXIT_USAGE;
        }
    };
    let (tx, rx) = channel();
    let engine = match Engine::new(&settings, tx, tokio::runtime::Handle::current()) {
        Ok(e) => e.with_config_path(cli.config.clone()),
        Err(e) => {
            eprintln!("error: {e}");
            return EXIT_USAGE;
        }
    };
    let mut driver = Driver {
        app: App::new(settings),
        engine,
        rx,
        mode,
        yes: cli.overrides.yes,
        progress: None,
        printer: printer.clone(),
        blocked: false,
    };
    driver.run(target).await
}

struct Driver {
    app: App,
    engine: Engine,
    rx: EventReceiver,
    mode: Mode,
    yes: bool,
    progress: Option<Bars>,
    printer: Printer,
    blocked: bool,
}

struct Bars {
    multi: MultiProgress,
    overall: ProgressBar,
    item: ProgressBar,
}

impl Driver {
    async fn run(&mut self, target: &str) -> u8 {
        self.act(Action::SetInput(target.to_string()));
        self.act(Action::Resolve);

        // Resolution (and automatic planning).
        loop {
            let Some(event) = self.rx.recv().await else {
                return EXIT_USAGE;
            };
            self.handle(event);
            match (&self.app.state.target.status, &self.app.state.run.phase) {
                (TargetStatus::Failed(_), _) => {
                    return if self.blocked {
                        EXIT_BLOCKED
                    } else {
                        EXIT_USAGE
                    };
                }
                (TargetStatus::Resolved, _) if self.mode == Mode::Info => {
                    self.print_info();
                    return 0;
                }
                (TargetStatus::Resolved, Phase::Ready) => break,
                (TargetStatus::Resolved, Phase::Idle) if self.app.state.plan.is_none() => {
                    // Planning failed; the log said why.
                    return EXIT_USAGE;
                }
                _ => {}
            }
        }

        self.print_plan();
        if self.mode == Mode::Plan {
            return 0;
        }

        // Download.
        if !self.app.state.settings.has_contact() {
            eprintln!(
                "error: a contact address is required; pass --contact or set it in the settings file"
            );
            return EXIT_USAGE;
        }
        if self.app.state.settings.variant.is_full_resolution() && !self.yes {
            eprintln!(
                "error: full resolution downloads are limited to 5 images per minute by the BnF; \
                 re-run with --yes to confirm the estimate above"
            );
            return EXIT_USAGE;
        }
        self.start_bars();
        self.act(Action::Start);
        let mut ctrl_c = Box::pin(tokio::signal::ctrl_c());
        loop {
            tokio::select! {
                event = self.rx.recv() => {
                    let Some(event) = event else { break };
                    self.handle(event);
                    if !self.app.state.run.phase.is_active() && self.app.state.run.progress.finished_at.is_some() {
                        break;
                    }
                }
                _ = &mut ctrl_c => {
                    self.app.log(LogLevel::Warn, "Interrupted: cancelling after the current views (Ctrl-C again to abort)");
                    self.act(Action::Cancel);
                    ctrl_c = Box::pin(tokio::signal::ctrl_c());
                }
            }
        }
        self.finish_bars();
        let run = &self.app.state.run;
        if self.blocked {
            EXIT_BLOCKED
        } else if run.progress.failed_items > 0 || matches!(run.phase, Phase::Failed(_)) {
            EXIT_PARTIAL
        } else {
            0
        }
    }

    fn act(&mut self, action: Action) {
        for cmd in self.app.update(action) {
            self.engine.execute(cmd);
        }
    }

    fn handle(&mut self, event: Event) {
        if matches!(event, Event::Blocked(_)) {
            self.blocked = true;
        }
        if let Event::Resolved(Err(e)) = &event
            && e.contains("403")
        {
            self.blocked = true;
        }
        for cmd in self.app.apply(event) {
            if let Command::SaveSettings(_) = cmd {
                continue;
            }
            self.engine.execute(cmd);
        }
        self.update_bars();
    }

    fn print_info(&self) {
        let Some(resolved) = &self.app.state.target.resolved else {
            return;
        };
        match resolved {
            Resolved::Periodical {
                ark,
                title,
                years,
                total_issues,
            } => {
                println!("Periodical  {}", ark.gallica_url());
                println!("Title       {}", title.as_deref().unwrap_or("?"));
                println!(
                    "Years       {} ({} years with issues online)",
                    match (years.first(), years.last()) {
                        (Some(a), Some(b)) => format!("{a}–{b}"),
                        _ => "none".into(),
                    },
                    years.len()
                );
                if let Some(n) = total_issues {
                    println!("Issues      {}", format::count(*n as u64));
                }
            }
            Resolved::Document {
                ark,
                title,
                date,
                typedoc,
                views,
                rights,
            } => {
                println!("Document    {}", ark.gallica_url());
                println!("Title       {}", title.as_deref().unwrap_or("?"));
                println!("Date        {}", date.as_deref().unwrap_or("?"));
                println!("Type        {}", typedoc.as_deref().unwrap_or("?"));
                println!("Views       {views}");
                if !rights.is_empty() {
                    println!("Rights      {}", rights.join(" / "));
                }
            }
        }
    }

    fn print_plan(&self) {
        let Some(plan) = &self.app.state.plan else {
            return;
        };
        let settings = &self.app.state.settings;
        let est = &plan.estimate;
        println!(
            "Plan: {} items, {}{} views as {} ({})",
            est.items,
            if est.views_known { "" } else { "about " },
            format::count(est.views),
            settings.variant,
            settings.variant.description()
        );
        println!(
            "      {} metadata + {} image{} requests, about {}{}",
            format::count(est.metadata_requests),
            format::count(est.image_requests),
            if est.text_requests > 0 {
                format!(" + {} text", est.text_requests)
            } else {
                String::new()
            },
            format::bytes(est.bytes_guess),
            match est.minimum {
                Some(d) if d > Duration::ZERO =>
                    format!(", at least {} under the BnF quotas", format::duration(d)),
                _ => String::new(),
            }
        );
        println!("      into {}", settings.output_dir.display());
        if self.mode == Mode::Plan {
            const MAX: usize = 60;
            for item in plan.items.iter().take(MAX) {
                println!(
                    "  {:<12} {:<16} {}",
                    item.date
                        .map(|d| d.to_string())
                        .or_else(|| item.year.map(|y| y.to_string()))
                        .unwrap_or_default(),
                    item.ark.name(),
                    item.label
                );
            }
            if plan.items.len() > MAX {
                println!("  ... {} more", plan.items.len() - MAX);
            }
        }
    }

    fn start_bars(&mut self) {
        let multi = MultiProgress::new();
        if !std::io::IsTerminal::is_terminal(&std::io::stderr()) {
            multi.set_draw_target(ProgressDrawTarget::hidden());
        }
        let overall = multi.add(ProgressBar::new(
            self.app.state.run.progress.views_total.unwrap_or(0),
        ));
        overall.set_style(
            ProgressStyle::with_template("{prefix:>8} [{bar:30}] {pos}/{len} views {msg}")
                .expect("valid template")
                .progress_chars("=> "),
        );
        overall.set_prefix("total");
        let item = multi.add(ProgressBar::new(0));
        item.set_style(
            ProgressStyle::with_template("{prefix:>8} [{bar:30}] {pos}/{len} {msg}")
                .expect("valid template")
                .progress_chars("=> "),
        );
        item.set_prefix("item");
        overall.enable_steady_tick(Duration::from_millis(250));
        self.printer.set_bars(Some(multi.clone()));
        self.progress = Some(Bars {
            multi,
            overall,
            item,
        });
    }

    fn update_bars(&mut self) {
        let Some(bars) = &self.progress else { return };
        let run = &self.app.state.run;
        let p = &run.progress;
        bars.overall.set_length(p.views_total.unwrap_or(0));
        bars.overall.set_position(p.views_done);
        let mut msg = format!(
            "{}/{} items, {}, {}",
            p.items_done,
            p.items_total,
            format::bytes(p.bytes),
            format::rate(p.rate())
        );
        if let Some(eta) = p.eta() {
            msg.push_str(&format!(", ETA {}", format::duration(eta)));
        }
        if let Some((class, left)) = p.current_quota_wait() {
            msg.push_str(&format!(", quota {class:?} {}", format::duration(left)));
        }
        bars.overall.set_message(msg);
        if let Some((i, it)) = run
            .items
            .iter()
            .enumerate()
            .find(|(_, it)| it.status == ItemStatus::Fetching)
        {
            let name = self
                .app
                .state
                .plan
                .as_ref()
                .and_then(|pl| pl.items.get(i))
                .map(|pi| {
                    pi.date
                        .map(|d| d.to_string())
                        .unwrap_or_else(|| pi.ark.name().to_string())
                })
                .unwrap_or_default();
            bars.item.set_length(it.total_views.unwrap_or(0) as u64);
            bars.item.set_position(it.done_views as u64);
            bars.item.set_message(name);
        }
    }

    fn finish_bars(&mut self) {
        if let Some(bars) = self.progress.take() {
            bars.item.finish_and_clear();
            bars.overall.finish_and_clear();
            let _ = bars.multi.clear();
            self.printer.set_bars(None);
        }
        let p = &self.app.state.run.progress;
        println!(
            "{}: {} items, {} views, {} in {}{}",
            self.app.state.run.phase.label(),
            p.items_done,
            p.views_done,
            format::bytes(p.bytes),
            format::duration(p.elapsed()),
            if p.failed_items > 0 {
                format!(", {} failed", p.failed_items)
            } else {
                String::new()
            }
        );
    }
}
