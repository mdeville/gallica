//! The panels, built with egui.

use std::path::PathBuf;
use std::sync::mpsc;

use egui::{Color32, RichText, Ui};
use egui_extras::{Column, TableBuilder};
use gallica_core::{Action, ItemStatus, LogLevel, Phase, TargetStatus, VariantSpec, format};

use crate::app::{GuiApp, warn};
use crate::state::{Confirm, parse_years};

const GREEN: Color32 = Color32::from_rgb(0x3c, 0xb0, 0x43);
const YELLOW: Color32 = Color32::from_rgb(0xd9, 0xa4, 0x00);
const RED: Color32 = Color32::from_rgb(0xd3, 0x3c, 0x3c);
const BLUE: Color32 = Color32::from_rgb(0x3a, 0x7b, 0xd5);

/// File and Help menus.
pub fn menu_bar(app: &mut GuiApp, ctx: &egui::Context) {
    egui::TopBottomPanel::top("menu").show(ctx, |ui| {
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button("File", |ui| {
                if ui.button("Open job…").clicked() {
                    ui.close();
                    pick_job(app);
                }
                if let Some(path) = gallica_core::Settings::config_path() {
                    ui.label(RichText::new(format!("Settings: {}", path.display())).small().weak());
                }
                ui.separator();
                if ui.button("Quit").clicked() {
                    ui.close();
                    app.request_quit();
                }
            });
            ui.menu_button("Help", |ui| {
                if ui.button("Keys and panels").clicked() {
                    ui.close();
                    app.gui.show_about = true;
                }
                if ui.button("BnF API quotas (web)").clicked() {
                    ui.close();
                    ctx.open_url(egui::OpenUrl::new_tab("https://api.bnf.fr/fr/node/232"));
                }
                if ui.button("Gallica conditions of use (web)").clicked() {
                    ui.close();
                    ctx.open_url(egui::OpenUrl::new_tab(
                        "https://gallica.bnf.fr/html/und/conditions-dutilisation-des-contenus-de-gallica",
                    ));
                }
            });
        });
    });
}

/// Target input and resolution status.
pub fn target_panel(app: &mut GuiApp, ctx: &egui::Context) {
    egui::TopBottomPanel::top("target").show(ctx, |ui| {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label("Target");
            let busy = app.core.state.run.phase.is_active();
            let response = ui.add_enabled(
                !busy,
                egui::TextEdit::singleline(&mut app.gui.buffers.target)
                    .hint_text("https://gallica.bnf.fr/ark:/12148/… or bpt6k…")
                    .desired_width(ui.available_width() - 90.0),
            );
            let submitted = response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if (ui
                .add_enabled(!busy, egui::Button::new("Resolve"))
                .clicked()
                || submitted)
                && !app.gui.buffers.target.trim().is_empty()
            {
                let text = app.gui.buffers.target.trim().to_string();
                app.dispatch(Action::SetInput(text));
                app.dispatch(Action::Resolve);
            }
        });
        ui.horizontal(|ui| {
            let state = &app.core.state;
            match &state.target.status {
                TargetStatus::Idle => {
                    ui.label(
                        RichText::new("Paste a Gallica URL or an ARK and press Enter.").weak(),
                    );
                }
                TargetStatus::Resolving => {
                    ui.spinner();
                    ui.label("Resolving…");
                }
                TargetStatus::Resolved => {
                    if let Some(r) = &state.target.resolved {
                        ui.colored_label(GREEN, "✓");
                        ui.label(RichText::new(r.title().unwrap_or("(no title)")).strong());
                        ui.label(RichText::new(r.summary()).weak());
                    }
                }
                TargetStatus::Failed(e) => {
                    ui.colored_label(RED, format!("✗ {e}"));
                }
            }
        });
        ui.add_space(4.0);
    });
}

/// Details and settings.
pub fn side_panel(app: &mut GuiApp, ctx: &egui::Context) {
    egui::SidePanel::left("side")
        .resizable(true)
        .default_width(340.0)
        .min_width(280.0)
        .show(ctx, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.heading("Details");
                details(app, ui);
                ui.add_space(8.0);
                ui.separator();
                ui.heading("Settings");
                settings(app, ui);
            });
        });
}

fn details(app: &GuiApp, ui: &mut Ui) {
    let state = &app.core.state;
    match &state.target.resolved {
        Some(r) => {
            ui.label(r.summary());
        }
        None => {
            ui.label(RichText::new("No target resolved yet.").weak());
            return;
        }
    }
    match (&state.plan, &state.run.phase) {
        (_, Phase::Planning) => {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Building the plan…");
            });
        }
        (Some(plan), _) => {
            let e = &plan.estimate;
            egui::Grid::new("estimate").num_columns(2).show(ui, |ui| {
                ui.label("Items");
                ui.label(format::count(e.items as u64));
                ui.end_row();
                ui.label("Views");
                ui.label(format!(
                    "{}{}",
                    if e.views_known { "" } else { "about " },
                    format::count(e.views)
                ));
                ui.end_row();
                ui.label("Requests");
                ui.label(format::count(
                    e.metadata_requests + e.image_requests + e.text_requests,
                ));
                ui.end_row();
                ui.label("Size");
                ui.label(format!("about {}", format::bytes(e.bytes_guess)));
                ui.end_row();
                ui.label("Minimum time");
                match e.minimum {
                    Some(d) if d.as_secs() > 0 => {
                        ui.colored_label(YELLOW, format!("{} (BnF quotas)", format::duration(d)));
                    }
                    _ => {
                        ui.label(RichText::new("no per-minute quota for this variant").weak());
                    }
                }
                ui.end_row();
            });
            ui.label(RichText::new(state.settings.variant.description()).weak());
        }
        (None, _) => {}
    }
}

fn settings(app: &mut GuiApp, ui: &mut Ui) {
    let busy = app.core.state.run.phase.is_active();
    ui.add_enabled_ui(!busy, |ui| {
        egui::Grid::new("settings")
            .num_columns(2)
            .spacing([8.0, 6.0])
            .show(ui, |ui| {
                // Years
                ui.label("Years");
                let r = ui.add(
                    egui::TextEdit::singleline(&mut app.gui.buffers.years)
                        .hint_text("all, 1921 or 1921-1925")
                        .desired_width(160.0),
                );
                if r.lost_focus() {
                    match parse_years(&app.gui.buffers.years) {
                        Ok(years) => {
                            if years != app.core.state.settings.years {
                                app.dispatch(Action::SetYears(years));
                            }
                        }
                        Err(e) => warn(app, e),
                    }
                }
                ui.end_row();

                // Max issues
                ui.label("Max issues");
                let r = ui.add(
                    egui::TextEdit::singleline(&mut app.gui.buffers.max_issues)
                        .hint_text("all")
                        .desired_width(160.0),
                );
                if r.lost_focus() {
                    let text = app.gui.buffers.max_issues.trim();
                    let value = if text.is_empty() {
                        Ok(None)
                    } else {
                        text.parse::<usize>()
                            .map(Some)
                            .map_err(|_| "max issues must be a number".to_string())
                    };
                    match value {
                        Ok(v) => {
                            if v != app.core.state.settings.max_issues {
                                app.dispatch(Action::SetMaxIssues(v));
                            }
                        }
                        Err(e) => warn(app, e),
                    }
                }
                ui.end_row();

                // Variant
                ui.label("Images");
                ui.vertical(|ui| {
                    let mut chosen = app.gui.buffers.variant;
                    for choice in VariantSpec::CHOICES {
                        let label = match choice {
                            VariantSpec::Width(_) => "width (px)".to_string(),
                            other => other.to_string(),
                        };
                        let selected = matches!(
                            (chosen, choice),
                            (VariantSpec::Width(_), VariantSpec::Width(_))
                        ) || chosen == choice;
                        ui.horizontal(|ui| {
                            if ui.radio(selected, label).clicked() {
                                chosen = choice;
                            }
                            if let VariantSpec::Width(_) = choice {
                                let r = ui.add(
                                    egui::TextEdit::singleline(&mut app.gui.buffers.width)
                                        .desired_width(60.0),
                                );
                                if r.lost_focus() && matches!(chosen, VariantSpec::Width(_)) {
                                    chosen = VariantSpec::Width(0); // re-read below
                                }
                            }
                        });
                    }
                    if let VariantSpec::Width(_) = chosen {
                        match app.gui.buffers.width.trim().parse::<u32>() {
                            Ok(w) if w > 0 => chosen = VariantSpec::Width(w),
                            _ => {
                                chosen = VariantSpec::Width(1000);
                                app.gui.buffers.width = "1000".into();
                            }
                        }
                    }
                    if chosen != app.core.state.settings.variant {
                        app.gui.buffers.variant = chosen;
                        app.dispatch(Action::SetVariant(chosen));
                    }
                    ui.label(
                        RichText::new(app.core.state.settings.variant.description())
                            .small()
                            .weak(),
                    );
                });
                ui.end_row();

                // Output dir
                ui.label("Output");
                ui.horizontal(|ui| {
                    let r = ui.add(
                        egui::TextEdit::singleline(&mut app.gui.buffers.output_dir)
                            .desired_width(160.0),
                    );
                    if r.lost_focus() {
                        let text = app.gui.buffers.output_dir.trim().to_string();
                        if !text.is_empty()
                            && std::path::Path::new(&text)
                                != app.core.state.settings.output_dir.as_path()
                        {
                            app.dispatch(Action::SetOutputDir(PathBuf::from(text)));
                        }
                    }
                    if ui.button("…").on_hover_text("Choose a folder").clicked() {
                        pick_folder(app);
                    }
                });
                ui.end_row();

                // Flags
                ui.label("Overwrite");
                let mut overwrite = app.core.state.settings.overwrite;
                if ui
                    .checkbox(&mut overwrite, "replace existing files")
                    .changed()
                {
                    app.dispatch(Action::SetOverwrite(overwrite));
                }
                ui.end_row();
                ui.label("Save text");
                let mut save_text = app.core.state.settings.save_text;
                if ui
                    .checkbox(&mut save_text, "also fetch the plain text (5/min)")
                    .changed()
                {
                    app.dispatch(Action::SetSaveText(save_text));
                }
                ui.end_row();

                // Contact
                ui.label("Contact");
                let r = ui.add(
                    egui::TextEdit::singleline(&mut app.gui.buffers.contact)
                        .hint_text("you@example.org (required)")
                        .desired_width(160.0),
                );
                if r.lost_focus() {
                    let text = app.gui.buffers.contact.trim().to_string();
                    if Some(&text) != app.core.state.settings.contact.as_ref() {
                        app.dispatch(Action::SetContact(text));
                    }
                }
                ui.end_row();
            });
    });
    if !app.core.state.settings.has_contact() {
        ui.colored_label(
            RED,
            "A contact address is required: it is sent to Gallica with every request.",
        );
    }
}

/// Queue table and thumbnails of the selected item.
pub fn queue_panel(app: &mut GuiApp, ctx: &egui::Context) {
    egui::CentralPanel::default().show(ctx, |ui| {
        let Some(plan) = app.core.state.plan.clone() else {
            ui.heading("Queue");
            ui.label(RichText::new("The plan appears here once a target is resolved.").weak());
            return;
        };
        ui.heading(format!("Queue ({} items)", plan.items.len()));
        let table_height = ui.available_height() * 0.7;
        let row_height = 20.0;
        let mut clicked = None;
        egui::ScrollArea::horizontal()
            .id_salt("queue")
            .max_height(table_height)
            .show(ui, |ui| {
                TableBuilder::new(ui)
                    .striped(true)
                    .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
                    .column(Column::exact(90.0))
                    .column(Column::exact(120.0))
                    .column(Column::exact(70.0))
                    .column(Column::exact(70.0))
                    .column(Column::remainder().at_least(120.0))
                    .header(row_height, |mut header| {
                        for h in ["date", "ark", "views", "state", "progress"] {
                            header.col(|ui| {
                                ui.strong(h);
                            });
                        }
                    })
                    .body(|body| {
                        body.rows(row_height, plan.items.len(), |mut row| {
                            let i = row.index();
                            let item = &plan.items[i];
                            let st = app.core.state.run.items.get(i).cloned().unwrap_or_default();
                            row.set_selected(app.gui.selected == Some(i));
                            let total = st.total_views.or(item.views);
                            row.col(|ui| {
                                ui.label(
                                    item.date
                                        .map(|d| d.to_string())
                                        .or_else(|| item.year.map(|y| y.to_string()))
                                        .unwrap_or_default(),
                                );
                            });
                            row.col(|ui| {
                                ui.label(item.ark.name());
                            });
                            row.col(|ui| {
                                ui.label(match total {
                                    Some(t) => format!("{}/{}", st.done_views, t),
                                    None => String::new(),
                                });
                            });
                            row.col(|ui| {
                                let color = match st.status {
                                    ItemStatus::Pending => ui.visuals().weak_text_color(),
                                    ItemStatus::Fetching => YELLOW,
                                    ItemStatus::Done => GREEN,
                                    ItemStatus::Skipped => BLUE,
                                    ItemStatus::Failed => RED,
                                };
                                ui.colored_label(color, st.status.label());
                            });
                            row.col(|ui| {
                                if let Some(t) = total.filter(|t| *t > 0) {
                                    ui.add(
                                        egui::ProgressBar::new(st.done_views as f32 / t as f32)
                                            .desired_width(ui.available_width().max(60.0)),
                                    );
                                }
                            });
                            if row.response().clicked() {
                                clicked = Some(i);
                            }
                        });
                    });
            });
        if let Some(i) = clicked {
            app.gui.selected = Some(i);
        }
        ui.separator();
        if let Some(i) = app.gui.selected
            && let (Some(item), Some(st)) = (plan.items.get(i), app.core.state.run.items.get(i))
        {
            ui.horizontal(|ui| {
                ui.strong(item.ark.name());
                ui.label(&item.label);
                if let Some(e) = &st.error {
                    ui.colored_label(RED, e);
                }
            });
            if matches!(st.status, ItemStatus::Done | ItemStatus::Skipped) {
                thumbnails(app, ui, &plan, i);
            }
        }
    });
}

fn thumbnails(app: &GuiApp, ui: &mut Ui, plan: &gallica_core::Plan, index: usize) {
    let Some(resolved) = &app.core.state.target.resolved else {
        return;
    };
    let item = &plan.items[index];
    let target_dir =
        gallica_core::layout::target_dir(&app.core.state.settings.output_dir, resolved);
    let dir = gallica_core::layout::item_dir(&target_dir, resolved, item);
    let variant = app.core.state.settings.image_variant();
    let total = app.core.state.run.items[index]
        .total_views
        .or(item.views)
        .unwrap_or(0)
        .min(40);
    egui::ScrollArea::horizontal()
        .id_salt("thumbs")
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                for view in 1..=total {
                    let path = dir.join(gallica_core::layout::view_file_name(&variant, view));
                    if path.is_file() {
                        let uri = format!("file://{}", path.display());
                        ui.add(egui::Image::new(uri).fit_to_exact_size(egui::vec2(90.0, 130.0)))
                            .on_hover_text(format!("f{view}"));
                    }
                }
            });
        });
}

/// Progress, buttons and log.
pub fn bottom_panel(app: &mut GuiApp, ctx: &egui::Context) {
    egui::TopBottomPanel::bottom("bottom")
        .resizable(true)
        .default_height(200.0)
        .show(ctx, |ui| {
            ui.add_space(4.0);
            progress(app, ui);
            ui.separator();
            ui.horizontal(|ui| {
                ui.heading("Log");
                ui.toggle_value(&mut app.gui.show_log, "show");
                if ui.small_button("clear").clicked() {
                    app.log.clear();
                }
            });
            if app.gui.show_log {
                egui::ScrollArea::vertical()
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        // Same lines as the headless commands print.
                        for entry in app.log.snapshot() {
                            let color = match entry.level {
                                LogLevel::Trace | LogLevel::Debug => ui.visuals().weak_text_color(),
                                LogLevel::Info if entry.target.is_some() => {
                                    ui.visuals().weak_text_color()
                                }
                                LogLevel::Info => ui.visuals().text_color(),
                                LogLevel::Warn => YELLOW,
                                LogLevel::Error => RED,
                            };
                            ui.colored_label(color, entry.line());
                        }
                    });
            }
        });
}

fn progress(app: &mut GuiApp, ui: &mut Ui) {
    let state = &app.core.state;
    let p = &state.run.progress;
    let phase = state.run.phase.clone();
    let mut text = phase.label().to_string();
    if p.items_total > 0 {
        text.push_str(&format!(
            "  ·  {}/{} items  ·  {}/{} views  ·  {}",
            p.items_done,
            p.items_total,
            p.views_done,
            p.views_total
                .map(|v| v.to_string())
                .unwrap_or_else(|| "?".into()),
            format::bytes(p.bytes)
        ));
        if phase.is_active() {
            text.push_str(&format!("  ·  {}", format::rate(p.rate())));
            if let Some(eta) = p.eta() {
                text.push_str(&format!("  ·  ETA {}", format::duration(eta)));
            }
            if let Some((class, left)) = p.current_quota_wait() {
                text.push_str(&format!("  ·  quota {class:?} {}", format::duration(left)));
            }
        } else if p.finished_at.is_some() {
            text.push_str(&format!("  ·  in {}", format::duration(p.elapsed())));
        }
        if p.failed_items > 0 {
            text.push_str(&format!("  ·  {} failed", p.failed_items));
        }
    }
    let fraction = p.fraction() as f32;
    let can_start = state.plan.is_some() && !phase.is_active() && phase != Phase::Planning;
    let has_failed = state
        .run
        .items
        .iter()
        .any(|i| i.status == ItemStatus::Failed);
    let full_res = state.settings.variant.is_full_resolution();
    ui.horizontal(|ui| {
        if ui
            .add_enabled(can_start, egui::Button::new("▶ Start"))
            .clicked()
        {
            if full_res {
                app.gui.confirm = Some(Confirm::StartFullRes);
            } else {
                app.dispatch(Action::Start);
            }
        }
        match phase {
            Phase::Running => {
                if ui.button("⏸ Pause").clicked() {
                    app.dispatch(Action::Pause);
                }
            }
            Phase::Paused => {
                if ui.button("▶ Resume").clicked() {
                    app.dispatch(Action::Resume);
                }
            }
            _ => {
                let _ = ui.add_enabled(false, egui::Button::new("⏸ Pause"));
            }
        }
        if ui
            .add_enabled(phase.is_active(), egui::Button::new("■ Cancel"))
            .clicked()
        {
            app.gui.confirm = Some(Confirm::Cancel);
        }
        if ui
            .add_enabled(
                has_failed && !phase.is_active(),
                egui::Button::new("↻ Retry failed"),
            )
            .clicked()
        {
            app.dispatch(Action::RetryFailed);
        }
    });
    let bar = egui::ProgressBar::new(fraction).text(text);
    let bar = match app.core.state.run.phase {
        Phase::Running => bar.fill(GREEN),
        Phase::Paused => bar.fill(YELLOW),
        Phase::Failed(_) => bar.fill(RED),
        _ => bar,
    };
    ui.add(bar);
}

/// Confirmation and about windows.
pub fn dialogs(app: &mut GuiApp, ctx: &egui::Context) {
    if let Some(confirm) = app.gui.confirm {
        let mut answer: Option<bool> = None;
        egui::Window::new("Confirm")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.set_max_width(420.0);
                ui.label(confirm.message());
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Yes").clicked() {
                        answer = Some(true);
                    }
                    if ui.button("No").clicked() {
                        answer = Some(false);
                    }
                });
            });
        if let Some(yes) = answer {
            app.gui.confirm = None;
            if yes {
                match confirm {
                    Confirm::StartFullRes => app.dispatch(Action::Start),
                    Confirm::QuitRunning => app.dispatch(Action::Quit),
                    Confirm::Cancel => app.dispatch(Action::Cancel),
                }
            }
        }
    }
    if app.gui.show_about {
        egui::Window::new("Gallica downloader")
            .collapsible(false)
            .open(&mut app.gui.show_about)
            .show(ctx, |ui| {
                ui.label("Paste a Gallica URL or ARK, check the plan and its cost in Details, adjust the settings, press Start.");
                ui.label("Downloads go to <output>/<ark> - <title>/<year>/<date> <ark>/f0001.jpg with a metadata.json per issue.");
                ui.label("Running the same target again resumes: finished issues are skipped without any request.");
                ui.label("The BnF quotas are enforced by the client; full resolution is limited to 5 images per minute.");
            });
    }
}

fn pick_folder(app: &mut GuiApp) {
    let (tx, rx) = mpsc::channel();
    let start = app.core.state.settings.output_dir.clone();
    app.handle.spawn(async move {
        let picked = rfd::AsyncFileDialog::new()
            .set_directory(&start)
            .pick_folder()
            .await
            .map(|f| f.path().to_path_buf());
        let _ = tx.send(picked);
    });
    app.gui.folder_picker = Some(rx);
}

fn pick_job(app: &mut GuiApp) {
    let (tx, rx) = mpsc::channel();
    let start = app.core.state.settings.output_dir.clone();
    app.handle.spawn(async move {
        let picked = rfd::AsyncFileDialog::new()
            .set_directory(&start)
            .add_filter("job file", &["json"])
            .pick_file()
            .await
            .map(|f| f.path().to_path_buf());
        let _ = tx.send(picked);
    });
    app.gui.job_picker = Some(rx);
}
