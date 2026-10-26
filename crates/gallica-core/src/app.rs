//! The pure state machine: actions and events in, commands out.

use std::time::{Duration, Instant, SystemTime};

use crate::messages::{Action, Command, Event, ItemOutcome};
use crate::settings::Settings;
use crate::state::{
    AppState, ItemId, ItemState, ItemStatus, LOG_CAPACITY, LogEntry, LogLevel, Phase, Progress,
    Run, TargetStatus,
};

/// The application.
#[derive(Debug, Clone, Default)]
pub struct App {
    /// The state front ends draw.
    pub state: AppState,
}

impl App {
    /// Creates an application with the given settings.
    pub fn new(settings: Settings) -> App {
        App {
            state: AppState {
                settings,
                ..Default::default()
            },
        }
    }

    /// Applies a user action.
    pub fn update(&mut self, action: Action) -> Vec<Command> {
        let s = &mut self.state;
        match action {
            Action::SetInput(text) => {
                if text != s.target.input {
                    s.target.input = text;
                    s.target.status = TargetStatus::Idle;
                }
                vec![]
            }
            Action::Resolve => {
                let input = s.target.input.trim().to_string();
                if input.is_empty() {
                    self.log(LogLevel::Warn, "Enter a Gallica URL or ARK first");
                    return vec![];
                }
                if s.run.phase.is_active() {
                    self.log(
                        LogLevel::Warn,
                        "A run is in progress; cancel it before resolving another target",
                    );
                    return vec![];
                }
                s.target.status = TargetStatus::Resolving;
                s.target.resolved = None;
                s.plan = None;
                s.run = Run {
                    phase: Phase::Resolving,
                    ..Default::default()
                };
                vec![Command::Resolve { input }]
            }
            Action::SetYears(years) => {
                let years = years.map(|(a, b)| if a <= b { (a, b) } else { (b, a) });
                self.change_settings(|st| st.years = years)
            }
            Action::SetMaxIssues(n) => {
                self.change_settings(|st| st.max_issues = n.filter(|n| *n > 0))
            }
            Action::SetVariant(v) => self.change_settings(|st| st.variant = v),
            Action::SetOutputDir(dir) => self.change_settings(|st| st.output_dir = dir),
            Action::SetOverwrite(b) => self.change_settings(|st| st.overwrite = b),
            Action::SetSaveText(b) => self.change_settings(|st| st.save_text = b),
            Action::SetContact(c) => {
                let c = c.trim().to_string();
                self.change_settings(|st| st.contact = if c.is_empty() { None } else { Some(c) })
            }
            Action::BuildPlan => self.build_plan(),
            Action::Start => self.start(None),
            Action::RetryFailed => {
                let failed: Vec<ItemId> = s
                    .run
                    .items
                    .iter()
                    .enumerate()
                    .filter(|(_, it)| it.status == ItemStatus::Failed)
                    .map(|(i, _)| i)
                    .collect();
                if failed.is_empty() {
                    self.log(LogLevel::Info, "No failed item to retry");
                    return vec![];
                }
                self.start(Some(failed))
            }
            Action::Pause => {
                if s.run.phase == Phase::Running {
                    s.run.phase = Phase::Paused;
                    self.log(LogLevel::Info, "Paused");
                    vec![Command::Pause]
                } else {
                    vec![]
                }
            }
            Action::Resume => {
                if s.run.phase == Phase::Paused {
                    s.run.phase = Phase::Running;
                    self.log(LogLevel::Info, "Resumed");
                    vec![Command::Resume]
                } else {
                    vec![]
                }
            }
            Action::Cancel => {
                if s.run.phase.is_active() {
                    s.run.cancel_requested = true;
                    self.log(LogLevel::Warn, "Cancelling after the current views...");
                    vec![Command::Cancel]
                } else {
                    vec![]
                }
            }
            Action::OpenJob(path) => vec![Command::OpenJob(path)],
            Action::ClearLog => {
                s.log.clear();
                vec![]
            }
            Action::Quit => {
                s.quit = true;
                let mut cmds = vec![Command::SaveSettings(s.settings.clone())];
                if s.run.phase.is_active() {
                    cmds.insert(0, Command::Cancel);
                }
                cmds.push(Command::Quit);
                cmds
            }
        }
    }

    /// Applies an engine event.
    pub fn apply(&mut self, event: Event) -> Vec<Command> {
        match event {
            Event::Resolved(Ok(resolved)) => {
                self.log(LogLevel::Info, format!("Resolved: {}", resolved.summary()));
                let s = &mut self.state;
                s.target.resolved = Some(resolved);
                s.target.status = TargetStatus::Resolved;
                s.run.phase = Phase::Idle;
                self.build_plan()
            }
            Event::Resolved(Err(e)) => {
                self.state.target.status = TargetStatus::Failed(e.clone());
                self.state.run.phase = Phase::Idle;
                self.log(
                    LogLevel::Error,
                    format!("Could not resolve the target: {e}"),
                );
                vec![]
            }
            Event::Planned(Ok(plan)) => {
                let est = &plan.estimate;
                let mut msg = format!(
                    "Plan: {} items, {}{} views, {} requests",
                    est.items,
                    if est.views_known { "" } else { "about " },
                    est.views,
                    est.metadata_requests + est.image_requests + est.text_requests
                );
                if let Some(min) = est.minimum {
                    msg.push_str(&format!(", at least {}", crate::format::duration(min)));
                }
                self.log(LogLevel::Info, msg);
                let s = &mut self.state;
                s.run = Run {
                    phase: Phase::Ready,
                    items: vec![ItemState::default(); plan.items.len()],
                    ..Default::default()
                };
                s.plan = Some(plan);
                vec![]
            }
            Event::Planned(Err(e)) => {
                self.state.plan = None;
                self.state.run.phase = Phase::Idle;
                self.log(LogLevel::Error, format!("Could not build the plan: {e}"));
                vec![]
            }
            Event::ItemStarted(id) => {
                if let Some(it) = self.state.run.items.get_mut(id) {
                    it.status = ItemStatus::Fetching;
                    it.error = None;
                }
                vec![]
            }
            Event::ItemViews { item, total } => {
                let s = &mut self.state;
                if let Some(it) = s.run.items.get_mut(item) {
                    it.total_views = Some(total);
                }
                s.run.progress.views_total = Some(recount_views(s));
                vec![]
            }
            Event::ViewDone { item, bytes, .. } => {
                let s = &mut self.state;
                if let Some(it) = s.run.items.get_mut(item) {
                    it.done_views += 1;
                    it.bytes += bytes;
                }
                s.run.progress.views_done += 1;
                s.run.progress.bytes += bytes;
                s.run.progress.quota_waiting = None;
                vec![]
            }
            Event::ViewFailed { item, view, error } => {
                let ark = self.item_ark(item);
                self.log(LogLevel::Warn, format!("{ark} f{view}: {error}"));
                vec![]
            }
            Event::ItemFinished { item, result } => {
                let s = &mut self.state;
                let mut message = None;
                if let Some(it) = s.run.items.get_mut(item) {
                    match result {
                        Ok(ItemOutcome::Done) => it.status = ItemStatus::Done,
                        Ok(ItemOutcome::Skipped { views }) => {
                            it.status = ItemStatus::Skipped;
                            it.total_views = Some(views);
                            it.done_views = views;
                            s.run.progress.views_done += views as u64;
                        }
                        Err(e) => {
                            it.status = ItemStatus::Failed;
                            it.error = Some(e.clone());
                            s.run.progress.failed_items += 1;
                            message = Some(e);
                        }
                    }
                }
                s.run.progress.items_done += 1;
                s.run.progress.views_total = Some(recount_views(s));
                if let Some(e) = message {
                    let ark = self.item_ark(item);
                    self.log(LogLevel::Error, format!("{ark}: {e}"));
                }
                vec![]
            }
            Event::QuotaWait { class, wait } => {
                let p = &mut self.state.run.progress;
                p.quota_waits += 1;
                p.quota_waiting = Some((class, wait, Instant::now()));
                if wait >= Duration::from_secs(5) {
                    self.log(
                        LogLevel::Info,
                        format!("Quota {class:?}: waiting {}", crate::format::duration(wait)),
                    );
                }
                vec![]
            }
            Event::Log(level, message) => {
                self.log(level, message);
                vec![]
            }
            Event::Blocked(message) => {
                self.state.run.phase = Phase::Failed(message.clone());
                self.log(LogLevel::Error, message);
                vec![]
            }
            Event::RunFinished { failed, cancelled } => {
                let s = &mut self.state;
                s.run.progress.finished_at = Some(Instant::now());
                s.run.progress.quota_waiting = None;
                if !matches!(s.run.phase, Phase::Failed(_)) {
                    s.run.phase = if cancelled {
                        Phase::Ready
                    } else {
                        Phase::Finished
                    };
                }
                s.run.cancel_requested = false;
                let p = &s.run.progress;
                let summary = format!(
                    "{}: {} items, {} views, {} in {}{}",
                    if cancelled { "Cancelled" } else { "Finished" },
                    p.items_done,
                    p.views_done,
                    crate::format::bytes(p.bytes),
                    crate::format::duration(p.elapsed()),
                    if failed > 0 {
                        format!(", {failed} failed")
                    } else {
                        String::new()
                    }
                );
                self.log(
                    if failed > 0 {
                        LogLevel::Warn
                    } else {
                        LogLevel::Info
                    },
                    summary,
                );
                vec![]
            }
            Event::JobLoaded(Ok(job)) => {
                let s = &mut self.state;
                s.target.input = job.input;
                s.target.resolved = Some(job.resolved);
                s.target.status = TargetStatus::Resolved;
                s.settings = job.settings;
                s.run = Run {
                    phase: Phase::Ready,
                    items: vec![ItemState::default(); job.plan.items.len()],
                    ..Default::default()
                };
                s.plan = Some(job.plan);
                self.log(LogLevel::Info, "Job loaded; Start resumes it");
                vec![]
            }
            Event::JobLoaded(Err(e)) => {
                self.log(LogLevel::Error, format!("Could not open the job: {e}"));
                vec![]
            }
        }
    }

    fn change_settings(&mut self, f: impl FnOnce(&mut Settings)) -> Vec<Command> {
        if self.state.run.phase.is_active() {
            self.log(LogLevel::Warn, "Settings cannot change during a run");
            return vec![];
        }
        let before = self.state.settings.clone();
        f(&mut self.state.settings);
        if self.state.settings == before {
            return vec![];
        }
        // The plan depends on years, max issues, variant and save_text.
        let plan_inputs_changed = before.years != self.state.settings.years
            || before.max_issues != self.state.settings.max_issues
            || before.variant != self.state.settings.variant
            || before.save_text != self.state.settings.save_text;
        if plan_inputs_changed && self.state.target.resolved.is_some() {
            return self.build_plan();
        }
        vec![]
    }

    fn build_plan(&mut self) -> Vec<Command> {
        let s = &mut self.state;
        let Some(resolved) = s.target.resolved.clone() else {
            return vec![];
        };
        if s.run.phase.is_active() {
            return vec![];
        }
        s.run.phase = Phase::Planning;
        vec![Command::BuildPlan {
            resolved,
            settings: s.settings.clone(),
        }]
    }

    fn start(&mut self, only: Option<Vec<ItemId>>) -> Vec<Command> {
        let s = &mut self.state;
        let (Some(resolved), Some(plan)) = (s.target.resolved.clone(), s.plan.clone()) else {
            self.log(LogLevel::Warn, "Resolve a target first");
            return vec![];
        };
        if s.run.phase.is_active() || s.run.phase == Phase::Planning {
            return vec![];
        }
        if !s.settings.has_contact() {
            self.log(
                LogLevel::Error,
                "Set a contact address in the settings first; it is sent to Gallica with every request",
            );
            return vec![];
        }
        let selected = only
            .clone()
            .unwrap_or_else(|| (0..plan.items.len()).collect());
        let mut items = vec![ItemState::default(); plan.items.len()];
        if only.is_some() {
            // Keep the state of the items that are not retried.
            for (i, it) in s.run.items.iter().enumerate() {
                if !selected.contains(&i) {
                    items[i] = it.clone();
                }
            }
        }
        let kept_done = items
            .iter()
            .filter(|it| it.status != ItemStatus::Pending)
            .count();
        let kept_views: u64 = items.iter().map(|it| it.done_views as u64).sum();
        s.run = Run {
            phase: Phase::Running,
            items,
            progress: Progress {
                items_total: plan.items.len(),
                items_done: kept_done,
                views_done: kept_views,
                views_total: Some(plan.estimate.views),
                started_at: Some(Instant::now()),
                ..Default::default()
            },
            cancel_requested: false,
        };
        let message = format!(
            "Starting: {} items as {} into {}",
            selected.len(),
            s.settings.variant,
            s.settings.output_dir.display()
        );
        self.log(LogLevel::Info, message);
        vec![
            Command::SaveSettings(self.state.settings.clone()),
            Command::StartRun {
                resolved,
                plan,
                settings: self.state.settings.clone(),
                only,
            },
        ]
    }

    fn item_ark(&self, item: ItemId) -> String {
        self.state
            .plan
            .as_ref()
            .and_then(|p| p.items.get(item))
            .map(|i| i.ark.name().to_string())
            .unwrap_or_else(|| format!("item {item}"))
    }

    /// Adds a log line.
    pub fn log(&mut self, level: LogLevel, message: impl Into<String>) {
        let log = &mut self.state.log;
        if log.len() >= LOG_CAPACITY {
            log.pop_front();
        }
        log.push_back(LogEntry {
            at: SystemTime::now(),
            level,
            message: message.into(),
        });
    }
}

/// Total views: known per-item totals, the plan's guess for the rest.
fn recount_views(s: &AppState) -> u64 {
    let Some(plan) = &s.plan else { return 0 };
    s.run
        .items
        .iter()
        .zip(&plan.items)
        .map(|(it, pi)| {
            it.total_views
                .or(pi.views)
                .unwrap_or(crate::plan::DEFAULT_VIEWS_PER_ISSUE) as u64
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::VariantSpec;
    use crate::state::{Estimate, Plan, PlanItem, Resolved};
    use gallica::{Ark, QuotaClass};

    fn resolved() -> Resolved {
        Resolved::Periodical {
            ark: Ark::new("cb327110547").unwrap(),
            title: Some("Les Belles images".into()),
            years: vec![1921, 1922],
            total_issues: Some(2),
        }
    }

    fn plan() -> Plan {
        let items: Vec<PlanItem> = (0..2)
            .map(|i| PlanItem {
                id: i,
                ark: Ark::new(format!("bpt6k96126{i}c")).unwrap(),
                date: None,
                year: Some(1921),
                label: String::new(),
                views: None,
            })
            .collect();
        Plan {
            estimate: Estimate {
                items: 2,
                views: 16,
                views_known: false,
                metadata_requests: 5,
                image_requests: 16,
                image_class: QuotaClass::HighResPreview,
                text_requests: 0,
                minimum: None,
                bytes_guess: 0,
            },
            items,
        }
    }

    fn ready_app() -> App {
        let mut app = App::new(Settings {
            contact: Some("me@example.org".into()),
            ..Settings::default()
        });
        app.update(Action::SetInput("cb327110547".into()));
        let cmds = app.update(Action::Resolve);
        assert!(matches!(cmds[0], Command::Resolve { .. }));
        let cmds = app.apply(Event::Resolved(Ok(resolved())));
        assert!(
            matches!(cmds[0], Command::BuildPlan { .. }),
            "auto-plans after resolve"
        );
        assert_eq!(app.state.run.phase, Phase::Planning);
        app.apply(Event::Planned(Ok(plan())));
        assert_eq!(app.state.run.phase, Phase::Ready);
        app
    }

    #[test]
    fn resolve_requires_input() {
        let mut app = App::new(Settings::default());
        assert!(app.update(Action::Resolve).is_empty());
        assert_eq!(app.state.log.back().unwrap().level, LogLevel::Warn);
    }

    #[test]
    fn start_requires_contact() {
        let mut app = ready_app();
        app.state.settings.contact = None;
        assert!(app.update(Action::Start).is_empty());
        assert_eq!(app.state.run.phase, Phase::Ready);
        assert_eq!(app.state.log.back().unwrap().level, LogLevel::Error);
    }

    #[test]
    fn full_run_flow() {
        let mut app = ready_app();
        let cmds = app.update(Action::Start);
        assert!(matches!(cmds[1], Command::StartRun { only: None, .. }));
        assert_eq!(app.state.run.phase, Phase::Running);
        assert_eq!(app.state.run.progress.views_total, Some(16));

        app.apply(Event::ItemStarted(0));
        app.apply(Event::ItemViews { item: 0, total: 3 });
        assert_eq!(app.state.run.progress.views_total, Some(3 + 8));
        for v in 1..=3 {
            app.apply(Event::ViewDone {
                item: 0,
                view: v,
                bytes: 100,
                skipped: false,
            });
        }
        app.apply(Event::ItemFinished {
            item: 0,
            result: Ok(ItemOutcome::Done),
        });
        assert_eq!(app.state.run.items[0].status, ItemStatus::Done);
        assert_eq!(app.state.run.progress.bytes, 300);

        app.apply(Event::ItemStarted(1));
        app.apply(Event::ItemFinished {
            item: 1,
            result: Err("boom".into()),
        });
        assert_eq!(app.state.run.items[1].status, ItemStatus::Failed);
        app.apply(Event::RunFinished {
            failed: 1,
            cancelled: false,
        });
        assert_eq!(app.state.run.phase, Phase::Finished);
        assert_eq!(app.state.run.progress.failed_items, 1);

        // Retry only the failed item, keeping the first one's state.
        let cmds = app.update(Action::RetryFailed);
        match &cmds[1] {
            Command::StartRun { only, .. } => assert_eq!(only, &Some(vec![1])),
            other => panic!("{other:?}"),
        }
        assert_eq!(app.state.run.items[0].status, ItemStatus::Done);
        assert_eq!(app.state.run.items[1].status, ItemStatus::Pending);
        assert_eq!(app.state.run.progress.items_done, 1);
    }

    #[test]
    fn pause_resume_cancel() {
        let mut app = ready_app();
        app.update(Action::Start);
        assert_eq!(app.update(Action::Pause), vec![Command::Pause]);
        assert_eq!(app.state.run.phase, Phase::Paused);
        assert!(app.update(Action::Pause).is_empty());
        assert_eq!(app.update(Action::Resume), vec![Command::Resume]);
        assert_eq!(app.update(Action::Cancel), vec![Command::Cancel]);
        assert!(app.state.run.cancel_requested);
        app.apply(Event::RunFinished {
            failed: 0,
            cancelled: true,
        });
        assert_eq!(app.state.run.phase, Phase::Ready);
        assert!(!app.state.run.cancel_requested);
    }

    #[test]
    fn settings_changes_replan_and_are_blocked_while_running() {
        let mut app = ready_app();
        let cmds = app.update(Action::SetVariant(VariantSpec::Full));
        assert!(matches!(cmds[0], Command::BuildPlan { .. }));
        app.apply(Event::Planned(Ok(plan())));
        assert!(
            app.update(Action::SetOverwrite(true)).is_empty(),
            "does not change the plan"
        );
        assert!(app.update(Action::SetYears(Some((1930, 1920)))).len() == 1);
        assert_eq!(app.state.settings.years, Some((1920, 1930)));
        app.apply(Event::Planned(Ok(plan())));

        app.update(Action::Start);
        assert!(
            app.update(Action::SetVariant(VariantSpec::MedRes))
                .is_empty()
        );
        assert_eq!(app.state.settings.variant, VariantSpec::Full);
    }

    #[test]
    fn blocked_marks_run_failed() {
        let mut app = ready_app();
        app.update(Action::Start);
        app.apply(Event::Blocked("challenge".into()));
        app.apply(Event::RunFinished {
            failed: 1,
            cancelled: false,
        });
        assert_eq!(app.state.run.phase, Phase::Failed("challenge".into()));
    }

    #[test]
    fn quit_saves_settings() {
        let mut app = ready_app();
        let cmds = app.update(Action::Quit);
        assert!(app.state.quit);
        assert!(matches!(cmds[0], Command::SaveSettings(_)));
        assert_eq!(cmds.last(), Some(&Command::Quit));
    }
}
