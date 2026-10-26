//! Drawing the six panels.

use gallica_core::{AppState, ItemStatus, LogLevel, Phase, TargetStatus, format};
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Clear, Gauge, Paragraph, Row, Table, Wrap};

use super::state::{Field, Focus, SettingsRow, UiState};

/// Draws the whole screen.
pub fn draw(frame: &mut Frame, ui: &mut UiState, app: &AppState) {
    let area = frame.area();
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(12),
            Constraint::Length(3),
            Constraint::Length(5),
            Constraint::Length(1),
        ])
        .split(area);
    draw_target(frame, rows[0], ui, app);

    let middle = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(42), Constraint::Percentage(58)])
        .split(rows[1]);
    let left = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(9)])
        .split(middle[0]);
    draw_details(frame, left[0], app);
    draw_settings(frame, left[1], ui, app);
    draw_queue(frame, middle[1], ui, app);
    draw_progress(frame, rows[2], app);
    draw_log(frame, rows[3], ui);
    draw_footer(frame, rows[4], ui, app);

    if let Some(confirm) = ui.confirm {
        draw_dialog(
            frame,
            area,
            "Confirm",
            &format!("{}\n\n[y] yes   [n] no", confirm.message()),
        );
    }
    if ui.show_help {
        draw_dialog(frame, area, "Keys", HELP);
    }
}

const HELP: &str = "\
Tab / Shift-Tab   move between panels
Enter             Target: edit the URL or ARK (Enter resolves)
                  Settings: edit / toggle / cycle the selected row
Up / Down         move in Settings, Queue and Log
Left / Right      Settings: previous / next image variant
s                 start the run        p   pause / resume
c                 cancel the run       r   retry failed items
x                 clear the log (Log panel)
?                 this help            q   quit

Any key closes this window.";

fn block(title: &str, focused: bool) -> Block<'_> {
    let b = Block::default()
        .borders(Borders::ALL)
        .title(format!(" {title} "));
    if focused {
        b.border_style(Style::default().fg(Color::Cyan))
    } else {
        b
    }
}

fn draw_target(frame: &mut Frame, area: Rect, ui: &UiState, app: &AppState) {
    let focused = ui.focus == Focus::Target;
    let editing = ui.edit.as_ref().filter(|e| e.field == Field::Target);
    let text = match editing {
        Some(e) => e.text.clone(),
        None => app.target.input.clone(),
    };
    let status: Span = match &app.target.status {
        TargetStatus::Idle if text.is_empty() => "Enter a Gallica URL or ARK".dim(),
        TargetStatus::Idle => "Enter to resolve".dim(),
        TargetStatus::Resolving => "resolving...".yellow(),
        TargetStatus::Resolved => match app.target.resolved.as_ref().and_then(|r| r.title()) {
            Some(t) => format!("✓ {t}").green(),
            None => "✓ resolved".green(),
        },
        TargetStatus::Failed(e) => format!("✗ {e}").red(),
    };
    let inner_width = area.width.saturating_sub(2) as usize;
    let status_width = status.content.chars().count() + 1;
    let field_width = inner_width.saturating_sub(status_width + 2).max(10);
    let shown: String = if editing.is_some() {
        text.clone()
    } else {
        text.chars().take(field_width).collect()
    };
    let line = Line::from(vec![
        "> ".into(),
        Span::raw(format!("{shown:<field_width$}")),
        " ".into(),
        status,
    ]);
    frame.render_widget(Paragraph::new(line).block(block("Target", focused)), area);
    if let Some(e) = editing {
        let x = area.x + 1 + 2 + e.cursor.min(field_width) as u16;
        frame.set_cursor_position((x.min(area.x + area.width - 2), area.y + 1));
    }
}

fn draw_details(frame: &mut Frame, area: Rect, app: &AppState) {
    let mut lines: Vec<Line> = Vec::new();
    match &app.target.resolved {
        Some(resolved) => {
            lines.push(Line::from(resolved.summary()));
        }
        None => lines.push("No target resolved yet.".dim().into()),
    }
    match (&app.plan, &app.run.phase) {
        (_, Phase::Planning) => lines.push("Building the plan...".yellow().into()),
        (Some(plan), _) => {
            let e = &plan.estimate;
            lines.push(Line::from(format!(
                "Plan: {} items, {}{} views",
                e.items,
                if e.views_known { "" } else { "~" },
                format::count(e.views)
            )));
            lines.push(Line::from(format!(
                "{} requests, ~{}",
                format::count(e.metadata_requests + e.image_requests + e.text_requests),
                format::bytes(e.bytes_guess)
            )));
            match e.minimum {
                Some(d) if d.as_secs() > 0 => lines.push(
                    Line::from(format!("At least {} under the quotas", format::duration(d)))
                        .yellow(),
                ),
                _ => lines.push("No per-minute quota for this variant".dim().into()),
            }
            lines.push(Line::from(app.settings.variant.description()).dim());
        }
        (None, _) => {}
    }
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: true })
            .block(block("Details", false)),
        area,
    );
}

fn draw_settings(frame: &mut Frame, area: Rect, ui: &UiState, app: &AppState) {
    let focused = ui.focus == Focus::Settings;
    let s = &app.settings;
    let mut lines: Vec<Line> = Vec::new();
    for (i, row) in SettingsRow::ALL.iter().enumerate() {
        let editing = ui.edit.as_ref().filter(|e| {
            matches!(
                (e.field, row),
                (Field::Years, SettingsRow::Years)
                    | (Field::MaxIssues, SettingsRow::MaxIssues)
                    | (Field::OutputDir, SettingsRow::OutputDir)
                    | (Field::Contact, SettingsRow::Contact)
                    | (Field::Width, SettingsRow::Variant)
            )
        });
        let value = match editing {
            Some(e) if e.field == Field::Width => format!("width:{}_", e.text),
            Some(e) => format!("{}_", e.text),
            None => match row {
                SettingsRow::Years => s
                    .years
                    .map(|(a, b)| {
                        if a == b {
                            a.to_string()
                        } else {
                            format!("{a}-{b}")
                        }
                    })
                    .unwrap_or_else(|| "all".into()),
                SettingsRow::MaxIssues => s
                    .max_issues
                    .map(|n| n.to_string())
                    .unwrap_or_else(|| "all".into()),
                SettingsRow::Variant => format!("◂ {} ▸", s.variant),
                SettingsRow::OutputDir => s.output_dir.display().to_string(),
                SettingsRow::Overwrite => if s.overwrite { "[x]" } else { "[ ]" }.to_string(),
                SettingsRow::SaveText => if s.save_text { "[x]" } else { "[ ]" }.to_string(),
                SettingsRow::Contact => s.contact.clone().unwrap_or_else(|| "required".into()),
            },
        };
        let selected = focused && ui.settings_row == i;
        let mut line = Line::from(vec![
            Span::raw(format!("{:<11}", row.label())).dim(),
            Span::raw(value),
        ]);
        if selected {
            line = line.style(Style::default().add_modifier(Modifier::REVERSED));
        }
        if *row == SettingsRow::Contact && !s.has_contact() {
            line = line.patch_style(Style::default().fg(Color::Red));
        }
        lines.push(line);
    }
    frame.render_widget(
        Paragraph::new(lines).block(block("Settings", focused)),
        area,
    );
}

fn draw_queue(frame: &mut Frame, area: Rect, ui: &mut UiState, app: &AppState) {
    let focused = ui.focus == Focus::Queue;
    let Some(plan) = &app.plan else {
        frame.render_widget(
            Paragraph::new("The plan appears here once a target is resolved.".dim())
                .block(block("Queue", focused)),
            area,
        );
        return;
    };
    if ui.table.selected().is_none() && !plan.items.is_empty() {
        ui.table.select(Some(0));
    }
    let bar_width = 10usize;
    let rows: Vec<Row> = plan
        .items
        .iter()
        .enumerate()
        .map(|(i, item)| {
            let st = app.run.items.get(i).cloned().unwrap_or_default();
            let date = item
                .date
                .map(|d| d.to_string())
                .or_else(|| item.year.map(|y| y.to_string()))
                .unwrap_or_default();
            let views = match st.total_views.or(item.views) {
                Some(t) => format!("{}/{}", st.done_views, t),
                None => String::new(),
            };
            let bar = match st.total_views.or(item.views) {
                Some(t) if t > 0 => {
                    let filled = (st.done_views as usize * bar_width / t as usize).min(bar_width);
                    format!("{}{}", "▇".repeat(filled), "░".repeat(bar_width - filled))
                }
                _ => String::new(),
            };
            let status_style = match st.status {
                ItemStatus::Pending => Style::default().dim(),
                ItemStatus::Fetching => Style::default().fg(Color::Yellow),
                ItemStatus::Done => Style::default().fg(Color::Green),
                ItemStatus::Skipped => Style::default().fg(Color::Blue),
                ItemStatus::Failed => Style::default().fg(Color::Red),
            };
            Row::new(vec![
                Cell::from(date),
                Cell::from(item.ark.name().to_string()),
                Cell::from(views),
                Cell::from(Span::styled(st.status.label(), status_style)),
                Cell::from(bar),
            ])
        })
        .collect();
    let header = Row::new(vec!["date", "ark", "views", "state", ""])
        .style(Style::default().add_modifier(Modifier::BOLD));
    let widths = [
        Constraint::Length(10),
        Constraint::Length(14),
        Constraint::Length(9),
        Constraint::Length(8),
        Constraint::Length(bar_width as u16),
    ];
    let title = format!("Queue ({} items)", plan.items.len());
    let error = ui
        .table
        .selected()
        .and_then(|i| app.run.items.get(i))
        .and_then(|it| it.error.clone());
    let (table_area, error_area) = match error {
        Some(_) => {
            let split = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Min(3), Constraint::Length(2)])
                .split(area);
            (split[0], Some(split[1]))
        }
        None => (area, None),
    };
    let table = Table::new(rows, widths)
        .header(header)
        .block(block(&title, focused))
        .row_highlight_style(Style::default().add_modifier(Modifier::REVERSED))
        .highlight_symbol("▶ ");
    frame.render_stateful_widget(table, table_area, &mut ui.table);
    if let (Some(area), Some(e)) = (error_area, error) {
        frame.render_widget(Paragraph::new(e).red().wrap(Wrap { trim: true }), area);
    }
}

fn draw_progress(frame: &mut Frame, area: Rect, app: &AppState) {
    let p = &app.run.progress;
    let phase = &app.run.phase;
    let mut label = phase.label().to_string();
    if p.items_total > 0 {
        label.push_str(&format!(
            "  {}/{} items  {}/{} views  {}",
            p.items_done,
            p.items_total,
            p.views_done,
            p.views_total
                .map(|v| v.to_string())
                .unwrap_or_else(|| "?".into()),
            format::bytes(p.bytes),
        ));
        if phase.is_active() {
            label.push_str(&format!("  {}", format::rate(p.rate())));
            if let Some(eta) = p.eta() {
                label.push_str(&format!("  ETA {}", format::duration(eta)));
            }
            if let Some((class, left)) = p.current_quota_wait() {
                label.push_str(&format!("  quota {class:?} {}", format::duration(left)));
            }
        } else if p.finished_at.is_some() {
            label.push_str(&format!("  in {}", format::duration(p.elapsed())));
        }
        if p.failed_items > 0 {
            label.push_str(&format!("  {} failed", p.failed_items));
        }
    }
    let color = match phase {
        Phase::Running => Color::Green,
        Phase::Paused => Color::Yellow,
        Phase::Failed(_) => Color::Red,
        _ => Color::Blue,
    };
    let gauge = Gauge::default()
        .block(block("Progress", false))
        .gauge_style(Style::default().fg(color).bg(Color::DarkGray))
        .ratio(p.fraction().clamp(0.0, 1.0))
        .label(label);
    frame.render_widget(gauge, area);
}

fn draw_log(frame: &mut Frame, area: Rect, ui: &UiState) {
    let focused = ui.focus == Focus::Log;
    let height = area.height.saturating_sub(2) as usize;
    let total = ui.log.len();
    let end = total.saturating_sub(ui.log_scroll);
    let start = end.saturating_sub(height);
    let lines: Vec<Line> = ui
        .log
        .iter()
        .skip(start)
        .take(end - start)
        .map(|entry| {
            let time = clock(entry.at);
            // Same layout as the headless commands' lines: level, then text.
            let label = format!("{:<5}", entry.level.label());
            let level = match entry.level {
                LogLevel::Trace | LogLevel::Debug => Span::raw(label).dark_gray(),
                LogLevel::Info => Span::raw(label).dim(),
                LogLevel::Warn => Span::raw(label).yellow(),
                LogLevel::Error => Span::raw(label).red(),
            };
            let text = Span::raw(entry.text());
            Line::from(vec![
                Span::raw(format!("{time} ")).dim(),
                level,
                Span::raw(" "),
                if entry.target.is_some() {
                    text.dim()
                } else {
                    text
                },
            ])
        })
        .collect();
    let title = if ui.log_scroll > 0 {
        format!("Log (↑{})", ui.log_scroll)
    } else {
        "Log".to_string()
    };
    frame.render_widget(Paragraph::new(lines).block(block(&title, focused)), area);
}

fn draw_footer(frame: &mut Frame, area: Rect, ui: &UiState, app: &AppState) {
    let keys: &str = if ui.edit.is_some() {
        "Enter apply · Esc cancel · Ctrl-U clear"
    } else {
        match (ui.focus, &app.run.phase) {
            (_, Phase::Running) => "p pause · c cancel · Tab focus · ? help · q quit",
            (_, Phase::Paused) => "p resume · c cancel · Tab focus · ? help · q quit",
            (Focus::Target, _) => "Enter edit target · Tab focus · s start · ? help · q quit",
            (Focus::Settings, _) => {
                "↑↓ row · Enter edit/toggle · ←→ variant · s start · ? help · q quit"
            }
            (Focus::Queue, _) => "↑↓ select · r retry failed · s start · ? help · q quit",
            (Focus::Log, _) => "↑↓ scroll · x clear · s start · ? help · q quit",
        }
    };
    frame.render_widget(Paragraph::new(Line::from(keys).dim()), area);
}

fn draw_dialog(frame: &mut Frame, area: Rect, title: &str, text: &str) {
    let lines: Vec<&str> = text.lines().collect();
    let width =
        (lines.iter().map(|l| l.chars().count()).max().unwrap_or(20) as u16 + 4).min(area.width);
    let height = (lines.len() as u16 + 2).min(area.height);
    let popup = Rect {
        x: area.x + (area.width.saturating_sub(width)) / 2,
        y: area.y + (area.height.saturating_sub(height)) / 2,
        width,
        height,
    };
    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(text)
            .wrap(Wrap { trim: false })
            .block(block(title, true)),
        popup,
    );
}

fn clock(at: std::time::SystemTime) -> String {
    let secs = at
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // Local time would need a time zone database; the log shows UTC.
    format!(
        "{:02}:{:02}:{:02}",
        (secs / 3600) % 24,
        (secs / 60) % 60,
        secs % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use gallica_core::gallica::document::Date;
    use gallica_core::gallica::{Ark, QuotaClass};
    use gallica_core::{Estimate, ItemState, Plan, PlanItem, Resolved, Run, Settings, Target};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn sample_state() -> AppState {
        let items: Vec<PlanItem> = (0..3)
            .map(|i| PlanItem {
                id: i,
                ark: Ark::new(format!("bpt6k96126{i}c")).unwrap(),
                date: Date::from_day_of_year(1921, 307 + 7 * i as u16),
                year: Some(1921),
                label: String::new(),
                views: None,
            })
            .collect();
        let mut run = Run {
            phase: Phase::Running,
            items: vec![ItemState::default(); 3],
            ..Default::default()
        };
        run.items[0] = ItemState {
            status: ItemStatus::Done,
            done_views: 8,
            total_views: Some(8),
            bytes: 800,
            error: None,
        };
        run.items[1] = ItemState {
            status: ItemStatus::Failed,
            done_views: 2,
            total_views: Some(8),
            bytes: 200,
            error: Some("boom".into()),
        };
        run.progress.items_total = 3;
        run.progress.items_done = 2;
        run.progress.views_total = Some(24);
        run.progress.views_done = 10;
        run.progress.bytes = 1000;
        run.progress.started_at = Some(std::time::Instant::now());
        AppState {
            target: Target {
                input: "https://gallica.bnf.fr/ark:/12148/cb327110547/".into(),
                status: TargetStatus::Resolved,
                resolved: Some(Resolved::Periodical {
                    ark: Ark::new("cb327110547").unwrap(),
                    title: Some("Les Belles images".into()),
                    years: vec![1905, 1936],
                    total_issues: Some(1186),
                }),
            },
            settings: Settings {
                contact: Some("me@example.org".into()),
                years: Some((1921, 1921)),
                ..Settings::default()
            },
            plan: Some(Plan {
                items,
                estimate: Estimate {
                    items: 3,
                    views: 24,
                    views_known: false,
                    metadata_requests: 7,
                    image_requests: 24,
                    image_class: QuotaClass::HighResPreview,
                    text_requests: 0,
                    minimum: None,
                    bytes_guess: 7_200_000,
                },
            }),
            run,
            ..Default::default()
        }
    }

    fn render(width: u16, height: u16, ui: &mut UiState, app: &AppState) -> String {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| draw(f, ui, app)).unwrap();
        let buffer = terminal.backend().buffer().clone();
        let mut out = String::new();
        for y in 0..buffer.area.height {
            for x in 0..buffer.area.width {
                out.push_str(buffer[(x, y)].symbol());
            }
            out.push('\n');
        }
        out
    }

    #[test]
    fn renders_all_panels() {
        let app = sample_state();
        let mut ui = UiState {
            focus: Focus::Queue,
            log: vec![
                gallica_core::LogEntry::new(LogLevel::Warn, "quota: waiting 12s"),
                gallica_core::LogEntry {
                    target: Some("gallica::client".into()),
                    ..gallica_core::LogEntry::new(LogLevel::Debug, "GET attempt=1")
                },
            ],
            ..Default::default()
        };
        ui.table.select(Some(1));
        let text = render(100, 32, &mut ui, &app);
        for needle in [
            "Target",
            "Details",
            "Settings",
            "Queue (3 items)",
            "Progress",
            "Log",
            "Les Belles images",
            "Periodical, 1905–1936, 1186 issues",
            "Plan: 3 items, ~24 views",
            "bpt6k961261c",
            "failed",
            "done",
            "boom",
            "Running",
            "10/24 views",
            "warn  quota: waiting 12s",
            "debug gallica::client: GET attempt=1",
            "me@example.org",
            "1921",
            "highres",
        ] {
            assert!(text.contains(needle), "missing {needle:?} in:\n{text}");
        }
    }

    #[test]
    fn renders_dialogs_and_small_terminal() {
        let app = sample_state();
        let mut ui = UiState {
            show_help: true,
            ..Default::default()
        };
        let text = render(80, 24, &mut ui, &app);
        assert!(text.contains("move between panels"), "{text}");
        ui.show_help = false;
        ui.confirm = Some(super::super::state::Confirm::Cancel);
        let text = render(80, 24, &mut ui, &app);
        assert!(text.contains("Cancel the run"), "{text}");
        // Does not panic on a tiny terminal.
        let _ = render(20, 8, &mut ui, &app);
    }

    #[test]
    fn renders_empty_state_with_edit() {
        let app = AppState::default();
        let mut ui = UiState {
            edit: Some(super::super::state::Edit::new(Field::Target, "cb3".into())),
            ..Default::default()
        };
        let text = render(80, 24, &mut ui, &app);
        assert!(text.contains("> cb3"), "{text}");
        assert!(text.contains("No target resolved yet"), "{text}");
        assert!(text.contains("required"), "{text}");
    }
}
