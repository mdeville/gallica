//! Executes commands against the SDK and reports events.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use futures::StreamExt;
use gallica::download::DownloadOptions;
use gallica::{Client, Error as SdkError, Resource};
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::job::{self, DoneMarker, ItemMetadata, JobFile, PeriodicalFile};
use crate::layout;
use crate::messages::{Command, Event, ItemOutcome};
use crate::plan;
use crate::settings::Settings;
use crate::state::{ItemId, LogLevel, Plan, PlanItem, Resolved};

/// Sender half of the event channel.
pub type EventSender = mpsc::UnboundedSender<Event>;
/// Receiver half of the event channel.
pub type EventReceiver = mpsc::UnboundedReceiver<Event>;

/// Creates the event channel.
pub fn channel() -> (EventSender, EventReceiver) {
    mpsc::unbounded_channel()
}

/// Runs commands on a tokio runtime and reports through the event channel.
pub struct Engine {
    client: Client,
    tx: EventSender,
    handle: tokio::runtime::Handle,
    pause: watch::Sender<bool>,
    cancel: CancellationToken,
    run: Option<JoinHandle<()>>,
}

impl Engine {
    /// Builds an engine from the settings. `handle` is the runtime work is
    /// spawned on.
    pub fn new(
        settings: &Settings,
        tx: EventSender,
        handle: tokio::runtime::Handle,
    ) -> Result<Engine, SdkError> {
        let hook_tx = tx.clone();
        let client = settings
            .client_builder()?
            .on_quota_wait(move |class, wait| {
                let _ = hook_tx.send(Event::QuotaWait { class, wait });
            })
            .build()?;
        Ok(Engine {
            client,
            tx,
            handle,
            pause: watch::channel(false).0,
            cancel: CancellationToken::new(),
            run: None,
        })
    }

    /// The SDK client.
    pub fn client(&self) -> &Client {
        &self.client
    }

    /// Executes one command. Never blocks.
    pub fn execute(&mut self, command: Command) {
        match command {
            Command::Resolve { input } => {
                let (client, tx) = (self.client.clone(), self.tx.clone());
                self.handle.spawn(async move {
                    let result = resolve(&client, &input).await.map_err(|e| e.to_string());
                    let _ = tx.send(Event::Resolved(result));
                });
            }
            Command::BuildPlan { resolved, settings } => {
                let (client, tx) = (self.client.clone(), self.tx.clone());
                self.handle.spawn(async move {
                    let result = build_plan(&client, &tx, &resolved, &settings)
                        .await
                        .map_err(|e| e.to_string());
                    let _ = tx.send(Event::Planned(result));
                });
            }
            Command::StartRun {
                resolved,
                plan,
                settings,
                only,
            } => {
                if let Some(previous) = self.run.take() {
                    previous.abort();
                }
                self.cancel = CancellationToken::new();
                let _ = self.pause.send(false);
                let ctx = RunContext {
                    client: self.client.clone(),
                    tx: self.tx.clone(),
                    pause: self.pause.subscribe(),
                    cancel: self.cancel.clone(),
                    resolved,
                    plan,
                    settings,
                    only,
                };
                self.run = Some(self.handle.spawn(run(ctx)));
            }
            Command::Pause => {
                let _ = self.pause.send(true);
            }
            Command::Resume => {
                let _ = self.pause.send(false);
            }
            Command::Cancel => {
                self.cancel.cancel();
                let _ = self.pause.send(false);
            }
            Command::OpenJob(path) => {
                let tx = self.tx.clone();
                self.handle.spawn(async move {
                    let result =
                        tokio::task::spawn_blocking(move || job::read_json::<JobFile>(&path))
                            .await
                            .map_err(|e| e.to_string())
                            .and_then(|r| r.map_err(|e| e.to_string()))
                            .and_then(|j| {
                                if j.version == job::JOB_VERSION {
                                    Ok(Box::new(j))
                                } else {
                                    Err(format!("unsupported job file version {}", j.version))
                                }
                            });
                    let _ = tx.send(Event::JobLoaded(result));
                });
            }
            Command::SaveSettings(settings) => {
                let tx = self.tx.clone();
                self.handle.spawn(async move {
                    let result = tokio::task::spawn_blocking(move || settings.save()).await;
                    let failure = match result {
                        Ok(Ok(_)) => None,
                        Ok(Err(e)) => Some(e.to_string()),
                        Err(e) => Some(e.to_string()),
                    };
                    if let Some(e) = failure {
                        let _ = tx.send(Event::Log(
                            LogLevel::Warn,
                            format!("Settings not saved: {e}"),
                        ));
                    }
                });
            }
            Command::Quit => {
                self.cancel.cancel();
                let _ = self.pause.send(false);
            }
        }
    }

    /// `true` while a run task is alive.
    pub fn is_running(&self) -> bool {
        self.run.as_ref().is_some_and(|h| !h.is_finished())
    }
}

// ---------------------------------------------------------------------------
// Resolve and plan
// ---------------------------------------------------------------------------

async fn resolve(client: &Client, input: &str) -> Result<Resolved, SdkError> {
    match Resource::parse(input)? {
        Resource::Periodical { ark } => {
            let years = client.issue_years(&ark).await?;
            // The Issues service gives no title: take it from the first issue.
            let mut title = None;
            if let Some(&year) = years.years.first()
                && let Some(first) = client.issues(&ark, year).await?.issues.first()
            {
                match client.oai_record(&first.ark).await {
                    Ok(rec) => title = rec.dc.title().map(str::to_string),
                    Err(e) => tracing::warn!(%ark, error = %e, "no title for the periodical"),
                }
            }
            Ok(Resolved::Periodical {
                ark,
                title,
                years: years.years,
                total_issues: years.total_issues,
            })
        }
        Resource::Document { ark, .. } => {
            let record = client.oai_record(&ark).await?;
            let views = match client.pagination(&ark).await {
                Ok(p) => p.view_count(),
                Err(e) => {
                    tracing::warn!(%ark, error = %e, "pagination failed, using the record");
                    record.dc.view_count().unwrap_or(0)
                }
            };
            Ok(Resolved::Document {
                title: record.dc.title().map(str::to_string),
                date: record.dc.date().map(str::to_string),
                typedoc: record.typedoc.clone(),
                views,
                rights: record.dc.rights.clone(),
                ark,
            })
        }
    }
}

async fn build_plan(
    client: &Client,
    tx: &EventSender,
    resolved: &Resolved,
    settings: &Settings,
) -> Result<Plan, SdkError> {
    let mut items = Vec::new();
    match resolved {
        Resolved::Periodical { ark, years, .. } => {
            let selected: Vec<u16> = years
                .iter()
                .copied()
                .filter(|y| settings.years.is_none_or(|(a, b)| (a..=b).contains(y)))
                .collect();
            if selected.is_empty() {
                return Err(SdkError::InvalidArgument(
                    "no year of this periodical is within the selected range".into(),
                ));
            }
            for year in selected {
                let list = client.issues(ark, year).await?;
                let _ = tx.send(Event::Log(
                    LogLevel::Info,
                    format!("{year}: {} issues", list.issues.len()),
                ));
                for issue in list.issues {
                    items.push(PlanItem {
                        id: items.len(),
                        ark: issue.ark.clone(),
                        date: issue.date(),
                        year: Some(issue.year),
                        label: issue.label,
                        views: None,
                    });
                }
                if settings.max_issues.is_some_and(|m| items.len() >= m) {
                    break;
                }
            }
        }
        Resolved::Document {
            ark,
            title,
            date,
            views,
            ..
        } => {
            items.push(PlanItem {
                id: 0,
                ark: ark.clone(),
                date: None,
                year: None,
                label: title.clone().or_else(|| date.clone()).unwrap_or_default(),
                views: Some(*views),
            });
        }
    }
    if let Some(max) = settings.max_issues {
        items.truncate(max);
        for (i, item) in items.iter_mut().enumerate() {
            item.id = i;
        }
    }
    let estimate = plan::estimate(resolved, &items, settings);
    Ok(Plan { items, estimate })
}

// ---------------------------------------------------------------------------
// Run
// ---------------------------------------------------------------------------

struct RunContext {
    client: Client,
    tx: EventSender,
    pause: watch::Receiver<bool>,
    cancel: CancellationToken,
    resolved: Resolved,
    plan: Plan,
    settings: Settings,
    only: Option<Vec<ItemId>>,
}

enum ItemError {
    /// Gallica refuses our requests; stop everything.
    Blocked(String),
    /// This item failed; continue with the next.
    Failed(String),
    /// The user cancelled.
    Cancelled,
}

impl From<SdkError> for ItemError {
    fn from(e: SdkError) -> Self {
        match blocked_message(&e) {
            Some(m) => ItemError::Blocked(m),
            None => ItemError::Failed(e.to_string()),
        }
    }
}

impl From<std::io::Error> for ItemError {
    fn from(e: std::io::Error) -> Self {
        ItemError::Failed(format!("file error: {e}"))
    }
}

/// Recognises the Cloudflare challenge and other hard refusals.
pub fn blocked_message(e: &SdkError) -> Option<String> {
    let status = e.status()?;
    if status == 403 {
        Some(
            "Gallica refused the request (HTTP 403). This is the bot challenge in front of gallica.bnf.fr; \
             automated clients are currently blocked. Nothing more can be downloaded until the BnF allows it."
                .into(),
        )
    } else {
        None
    }
}

async fn wait_if_paused(pause: &mut watch::Receiver<bool>, cancel: &CancellationToken) {
    while *pause.borrow() && !cancel.is_cancelled() {
        tokio::select! {
            _ = pause.changed() => {}
            _ = cancel.cancelled() => {}
        }
    }
}

async fn run(mut ctx: RunContext) {
    let target_dir = layout::target_dir(&ctx.settings.output_dir, &ctx.resolved);
    if let Err(e) = prepare_target_dir(&ctx, &target_dir) {
        let _ = ctx.tx.send(Event::Log(
            LogLevel::Error,
            format!("{}: {e}", target_dir.display()),
        ));
        let _ = ctx.tx.send(Event::RunFinished {
            failed: 0,
            cancelled: false,
        });
        return;
    }
    let _ = ctx.tx.send(Event::Log(
        LogLevel::Info,
        format!("Writing to {}", target_dir.display()),
    ));

    let selected: Vec<PlanItem> = ctx
        .plan
        .items
        .iter()
        .filter(|i| ctx.only.as_ref().is_none_or(|only| only.contains(&i.id)))
        .cloned()
        .collect();

    let mut failed = 0u32;
    let mut cancelled = false;
    for item in &selected {
        if ctx.cancel.is_cancelled() {
            cancelled = true;
            break;
        }
        wait_if_paused(&mut ctx.pause, &ctx.cancel).await;
        if ctx.cancel.is_cancelled() {
            cancelled = true;
            break;
        }
        let _ = ctx.tx.send(Event::ItemStarted(item.id));
        let dir = layout::item_dir(&target_dir, &ctx.resolved, item);
        match process_item(&mut ctx, item, &dir).await {
            Ok(outcome) => {
                let _ = ctx.tx.send(Event::ItemFinished {
                    item: item.id,
                    result: Ok(outcome),
                });
            }
            Err(ItemError::Failed(msg)) => {
                failed += 1;
                let _ = ctx.tx.send(Event::ItemFinished {
                    item: item.id,
                    result: Err(msg),
                });
            }
            Err(ItemError::Blocked(msg)) => {
                failed += 1;
                let _ = ctx.tx.send(Event::ItemFinished {
                    item: item.id,
                    result: Err("blocked by Gallica".into()),
                });
                let _ = ctx.tx.send(Event::Blocked(msg));
                break;
            }
            Err(ItemError::Cancelled) => {
                let _ = ctx.tx.send(Event::ItemFinished {
                    item: item.id,
                    result: Err("cancelled".into()),
                });
                cancelled = true;
                break;
            }
        }
    }
    let _ = ctx.tx.send(Event::RunFinished { failed, cancelled });
}

fn prepare_target_dir(ctx: &RunContext, target_dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(target_dir)?;
    if let Resolved::Periodical {
        ark,
        title,
        years,
        total_issues,
    } = &ctx.resolved
    {
        job::write_json(
            &target_dir.join(layout::PERIODICAL_FILE),
            &PeriodicalFile {
                ark: ark.clone(),
                title: title.clone(),
                years: years.clone(),
                total_issues: *total_issues,
            },
        )?;
    }
    job::write_json(
        &target_dir.join(layout::JOB_FILE),
        &JobFile {
            version: job::JOB_VERSION,
            input: ctx.resolved.ark().gallica_url().to_string(),
            resolved: ctx.resolved.clone(),
            settings: ctx.settings.clone(),
            plan: ctx.plan.clone(),
            saved_at: job::now_unix(),
        },
    )
}

async fn process_item(
    ctx: &mut RunContext,
    item: &PlanItem,
    dir: &Path,
) -> Result<ItemOutcome, ItemError> {
    std::fs::create_dir_all(dir)?;
    let done_path = dir.join(layout::DONE_FILE);
    if !ctx.settings.overwrite
        && let Ok(marker) = job::read_json::<DoneMarker>(&done_path)
        && marker.variant == ctx.settings.variant
        && (marker.text || !ctx.settings.save_text)
    {
        return Ok(ItemOutcome::Skipped {
            views: marker.views,
        });
    }

    let record = ctx.client.oai_record(&item.ark).await?;
    let pagination = ctx.client.pagination(&item.ark).await?;
    job::write_json(
        &dir.join(layout::METADATA_FILE),
        &ItemMetadata {
            ark: item.ark.clone(),
            record,
            pagination: pagination.clone(),
        },
    )?;
    let views: Vec<u32> = if pagination.pages.is_empty() {
        (1..=pagination.view_count()).collect()
    } else {
        pagination.pages.iter().map(|p| p.order).collect()
    };
    let _ = ctx.tx.send(Event::ItemViews {
        item: item.id,
        total: views.len() as u32,
    });

    let variant = ctx.settings.image_variant();
    let options = DownloadOptions::new().overwrite(ctx.settings.overwrite);
    let client = ctx.client.clone();
    let ark = item.ark.clone();
    let dir_buf: PathBuf = dir.to_path_buf();
    let cancel = ctx.cancel.clone();
    let pause = Arc::new(tokio::sync::Mutex::new(ctx.pause.clone()));

    let mut failures = 0u32;
    let mut blocked = None;
    let mut stream = futures::stream::iter(views)
        .map(|view| {
            let (client, ark, dir, options, cancel, pause) = (
                client.clone(),
                ark.clone(),
                dir_buf.clone(),
                options.clone(),
                cancel.clone(),
                pause.clone(),
            );
            async move {
                {
                    let mut p = pause.lock().await;
                    wait_if_paused(&mut p, &cancel).await;
                }
                if cancel.is_cancelled() {
                    return (view, Err(ItemError::Cancelled));
                }
                let path = dir.join(layout::view_file_name(&variant, view));
                let result = client
                    .download_view(&ark, view, &variant, &path, &options)
                    .await;
                (view, result.map_err(ItemError::from))
            }
        })
        .buffer_unordered(4);
    while let Some((view, result)) = stream.next().await {
        match result {
            Ok(out) => {
                let _ = ctx.tx.send(Event::ViewDone {
                    item: item.id,
                    view,
                    bytes: if out.skipped { 0 } else { out.bytes },
                    skipped: out.skipped,
                });
            }
            Err(ItemError::Cancelled) => {
                drop(stream);
                return Err(ItemError::Cancelled);
            }
            Err(ItemError::Blocked(m)) => {
                blocked = Some(m);
                break;
            }
            Err(ItemError::Failed(error)) => {
                failures += 1;
                let _ = ctx.tx.send(Event::ViewFailed {
                    item: item.id,
                    view,
                    error,
                });
            }
        }
    }
    drop(stream);
    if let Some(m) = blocked {
        return Err(ItemError::Blocked(m));
    }

    let mut text_saved = false;
    if ctx.settings.save_text && failures == 0 {
        let text_path = dir.join(layout::TEXT_FILE);
        if ctx.settings.overwrite || !text_path.exists() {
            match ctx.client.text(&item.ark, None).await {
                Ok(text) => {
                    std::fs::write(&text_path, text)?;
                    text_saved = true;
                }
                Err(e) => {
                    if let Some(m) = blocked_message(&e) {
                        return Err(ItemError::Blocked(m));
                    }
                    let _ = ctx.tx.send(Event::Log(
                        LogLevel::Warn,
                        format!("{}: text not saved: {e}", item.ark.name()),
                    ));
                }
            }
        } else {
            text_saved = true;
        }
    }

    if failures > 0 {
        return Err(ItemError::Failed(format!("{failures} views failed")));
    }
    job::write_json(
        &done_path,
        &DoneMarker {
            ark: item.ark.clone(),
            variant: ctx.settings.variant,
            views: pagination.pages.len().max(pagination.view_count() as usize) as u32,
            text: text_saved,
            finished_at: job::now_unix(),
        },
    )?;
    Ok(ItemOutcome::Done)
}
