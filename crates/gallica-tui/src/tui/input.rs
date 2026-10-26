//! Keys to actions.

use std::path::PathBuf;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use gallica_core::{Action, AppState, Phase, VariantSpec};

use super::state::{Confirm, Edit, Field, Focus, SettingsRow, UiState};

/// Translates a key press into actions, updating the UI state.
pub fn handle_key(key: KeyEvent, ui: &mut UiState, app: &AppState) -> Vec<Action> {
    if ui.show_help {
        ui.show_help = false;
        return vec![];
    }
    if let Some(confirm) = ui.confirm {
        return handle_confirm(key, confirm, ui, app);
    }
    if ui.edit.is_some() {
        return handle_edit(key, ui, app);
    }

    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    match key.code {
        KeyCode::Char('c') if ctrl => return quit(ui, app),
        KeyCode::Char('q') => return quit(ui, app),
        KeyCode::Char('?') | KeyCode::F(1) => {
            ui.show_help = true;
            return vec![];
        }
        KeyCode::Tab => {
            ui.focus = ui.focus.next();
            return vec![];
        }
        KeyCode::BackTab => {
            ui.focus = ui.focus.previous();
            return vec![];
        }
        KeyCode::Char('s') => return start(ui, app),
        KeyCode::Char('p') => {
            return match app.run.phase {
                Phase::Running => vec![Action::Pause],
                Phase::Paused => vec![Action::Resume],
                _ => vec![],
            };
        }
        KeyCode::Char('c') => {
            if app.run.phase.is_active() {
                ui.confirm = Some(Confirm::Cancel);
            }
            return vec![];
        }
        KeyCode::Char('r') => return vec![Action::RetryFailed],
        _ => {}
    }

    match ui.focus {
        Focus::Target => match key.code {
            KeyCode::Enter | KeyCode::Char('e') => {
                ui.edit = Some(Edit::new(Field::Target, app.target.input.clone()));
                vec![]
            }
            _ => vec![],
        },
        Focus::Settings => handle_settings_key(key, ui, app),
        Focus::Queue => {
            let len = app.plan.as_ref().map(|p| p.items.len()).unwrap_or(0);
            if len == 0 {
                return vec![];
            }
            let current = ui.table.selected().unwrap_or(0);
            let next = match key.code {
                KeyCode::Up | KeyCode::Char('k') => current.saturating_sub(1),
                KeyCode::Down | KeyCode::Char('j') => (current + 1).min(len - 1),
                KeyCode::PageUp => current.saturating_sub(10),
                KeyCode::PageDown => (current + 10).min(len - 1),
                KeyCode::Home => 0,
                KeyCode::End => len - 1,
                _ => current,
            };
            ui.table.select(Some(next));
            vec![]
        }
        Focus::Log => match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                ui.log_scroll = (ui.log_scroll + 1).min(ui.log.len().saturating_sub(1));
                vec![]
            }
            KeyCode::Down | KeyCode::Char('j') => {
                ui.log_scroll = ui.log_scroll.saturating_sub(1);
                vec![]
            }
            KeyCode::PageUp => {
                ui.log_scroll = (ui.log_scroll + 10).min(ui.log.len().saturating_sub(1));
                vec![]
            }
            KeyCode::PageDown => {
                ui.log_scroll = ui.log_scroll.saturating_sub(10);
                vec![]
            }
            KeyCode::End => {
                ui.log_scroll = 0;
                vec![]
            }
            KeyCode::Char('x') => {
                ui.log_scroll = 0;
                ui.log_buffer.clear();
                ui.log.clear();
                vec![]
            }
            _ => vec![],
        },
    }
}

fn quit(ui: &mut UiState, app: &AppState) -> Vec<Action> {
    if app.run.phase.is_active() {
        ui.confirm = Some(Confirm::QuitRunning);
        vec![]
    } else {
        vec![Action::Quit]
    }
}

fn start(ui: &mut UiState, app: &AppState) -> Vec<Action> {
    if app.plan.is_none() || app.run.phase.is_active() {
        return vec![];
    }
    if app.settings.variant.is_full_resolution() {
        ui.confirm = Some(Confirm::StartFullRes);
        vec![]
    } else {
        vec![Action::Start]
    }
}

fn handle_confirm(
    key: KeyEvent,
    confirm: Confirm,
    ui: &mut UiState,
    _app: &AppState,
) -> Vec<Action> {
    match key.code {
        KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
            ui.confirm = None;
            match confirm {
                Confirm::StartFullRes => vec![Action::Start],
                Confirm::QuitRunning => vec![Action::Quit],
                Confirm::Cancel => vec![Action::Cancel],
            }
        }
        KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc | KeyCode::Char('q') => {
            ui.confirm = None;
            vec![]
        }
        _ => vec![],
    }
}

fn handle_edit(key: KeyEvent, ui: &mut UiState, app: &AppState) -> Vec<Action> {
    let Some(edit) = ui.edit.as_mut() else {
        return vec![];
    };
    match key.code {
        KeyCode::Esc => {
            ui.edit = None;
            vec![]
        }
        KeyCode::Enter => {
            let edit = ui.edit.take().expect("checked above");
            commit_edit(edit, ui, app)
        }
        KeyCode::Backspace => {
            edit.backspace();
            vec![]
        }
        KeyCode::Delete => {
            edit.delete();
            vec![]
        }
        KeyCode::Left => {
            edit.left();
            vec![]
        }
        KeyCode::Right => {
            edit.right();
            vec![]
        }
        KeyCode::Home => {
            edit.home();
            vec![]
        }
        KeyCode::End => {
            edit.end();
            vec![]
        }
        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            edit.clear();
            vec![]
        }
        KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
            edit.insert(c);
            vec![]
        }
        _ => vec![],
    }
}

fn commit_edit(edit: Edit, ui: &mut UiState, app: &AppState) -> Vec<Action> {
    let text = edit.text.trim().to_string();
    match edit.field {
        Field::Target => {
            if text.is_empty() {
                return vec![];
            }
            vec![Action::SetInput(text), Action::Resolve]
        }
        Field::Years => {
            if text.is_empty() {
                return vec![Action::SetYears(None)];
            }
            match parse_years(&text) {
                Some(range) => vec![Action::SetYears(Some(range))],
                None => {
                    // Keep editing on invalid input.
                    ui.edit = Some(Edit::new(Field::Years, edit.text));
                    vec![]
                }
            }
        }
        Field::MaxIssues => {
            if text.is_empty() {
                return vec![Action::SetMaxIssues(None)];
            }
            match text.parse::<usize>() {
                Ok(n) => vec![Action::SetMaxIssues(Some(n))],
                Err(_) => {
                    ui.edit = Some(Edit::new(Field::MaxIssues, edit.text));
                    vec![]
                }
            }
        }
        Field::OutputDir => {
            if text.is_empty() {
                return vec![];
            }
            vec![Action::SetOutputDir(PathBuf::from(expand_home(&text)))]
        }
        Field::Contact => vec![Action::SetContact(text)],
        Field::Width => match text.parse::<u32>() {
            Ok(w) if w > 0 => vec![Action::SetVariant(VariantSpec::Width(w))],
            _ => {
                let _ = app;
                ui.edit = Some(Edit::new(Field::Width, edit.text));
                vec![]
            }
        },
    }
}

fn handle_settings_key(key: KeyEvent, ui: &mut UiState, app: &AppState) -> Vec<Action> {
    let rows = SettingsRow::ALL.len();
    match key.code {
        KeyCode::Up | KeyCode::Char('k') => {
            ui.settings_row = ui.settings_row.saturating_sub(1);
            vec![]
        }
        KeyCode::Down | KeyCode::Char('j') => {
            ui.settings_row = (ui.settings_row + 1).min(rows - 1);
            vec![]
        }
        KeyCode::Left | KeyCode::Char('h') => cycle_variant_if_selected(ui, app, -1),
        KeyCode::Right | KeyCode::Char('l') => cycle_variant_if_selected(ui, app, 1),
        KeyCode::Enter | KeyCode::Char(' ') | KeyCode::Char('e') => match ui.settings_row() {
            SettingsRow::Years => {
                let text = app
                    .settings
                    .years
                    .map(|(a, b)| {
                        if a == b {
                            a.to_string()
                        } else {
                            format!("{a}-{b}")
                        }
                    })
                    .unwrap_or_default();
                ui.edit = Some(Edit::new(Field::Years, text));
                vec![]
            }
            SettingsRow::MaxIssues => {
                let text = app
                    .settings
                    .max_issues
                    .map(|n| n.to_string())
                    .unwrap_or_default();
                ui.edit = Some(Edit::new(Field::MaxIssues, text));
                vec![]
            }
            SettingsRow::Variant => {
                if let VariantSpec::Width(w) = app.settings.variant {
                    ui.edit = Some(Edit::new(Field::Width, w.to_string()));
                    vec![]
                } else {
                    cycle_variant_if_selected(ui, app, 1)
                }
            }
            SettingsRow::OutputDir => {
                ui.edit = Some(Edit::new(
                    Field::OutputDir,
                    app.settings.output_dir.display().to_string(),
                ));
                vec![]
            }
            SettingsRow::Overwrite => vec![Action::SetOverwrite(!app.settings.overwrite)],
            SettingsRow::SaveText => vec![Action::SetSaveText(!app.settings.save_text)],
            SettingsRow::Contact => {
                ui.edit = Some(Edit::new(
                    Field::Contact,
                    app.settings.contact.clone().unwrap_or_default(),
                ));
                vec![]
            }
        },
        _ => vec![],
    }
}

fn cycle_variant_if_selected(ui: &UiState, app: &AppState, direction: i32) -> Vec<Action> {
    if ui.settings_row() != SettingsRow::Variant {
        return vec![];
    }
    let choices = VariantSpec::CHOICES;
    let current = choices
        .iter()
        .position(|v| match (v, app.settings.variant) {
            (VariantSpec::Width(_), VariantSpec::Width(_)) => true,
            (a, b) => *a == b,
        })
        .unwrap_or(0) as i32;
    let next = (current + direction).rem_euclid(choices.len() as i32) as usize;
    vec![Action::SetVariant(choices[next])]
}

fn parse_years(s: &str) -> Option<(u16, u16)> {
    let (a, b) = match s.split_once('-') {
        Some((a, b)) => (a.trim(), b.trim()),
        None => (s, s),
    };
    let a: u16 = a.parse().ok()?;
    let b: u16 = b.parse().ok()?;
    Some(if a <= b { (a, b) } else { (b, a) })
}

fn expand_home(path: &str) -> String {
    if let Some(rest) = path.strip_prefix("~/")
        && let Some(home) = directories_home()
    {
        return format!("{}/{rest}", home.trim_end_matches('/'));
    }
    path.to_string()
}

fn directories_home() -> Option<String> {
    std::env::var("HOME").ok().filter(|h| !h.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use gallica_core::gallica::{Ark, QuotaClass};
    use gallica_core::{Estimate, Plan, Settings};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn ready_state() -> AppState {
        AppState {
            settings: Settings {
                contact: Some("me@example.org".into()),
                ..Settings::default()
            },
            plan: Some(Plan {
                items: vec![],
                estimate: Estimate {
                    items: 0,
                    views: 0,
                    views_known: true,
                    metadata_requests: 0,
                    image_requests: 0,
                    image_class: QuotaClass::Default,
                    text_requests: 0,
                    minimum: None,
                    bytes_guess: 0,
                },
            }),
            run: gallica_core::Run {
                phase: Phase::Ready,
                ..Default::default()
            },
            ..Default::default()
        }
    }

    #[test]
    fn target_edit_resolves() {
        let mut ui = UiState::default();
        let app = AppState::default();
        assert!(handle_key(key(KeyCode::Enter), &mut ui, &app).is_empty());
        assert!(ui.edit.is_some());
        for c in "cb327110547".chars() {
            handle_key(key(KeyCode::Char(c)), &mut ui, &app);
        }
        let actions = handle_key(key(KeyCode::Enter), &mut ui, &app);
        assert_eq!(
            actions,
            vec![Action::SetInput("cb327110547".into()), Action::Resolve]
        );
        assert!(ui.edit.is_none());
    }

    #[test]
    fn start_asks_for_full_resolution() {
        let mut ui = UiState::default();
        let mut app = ready_state();
        assert_eq!(
            handle_key(key(KeyCode::Char('s')), &mut ui, &app),
            vec![Action::Start]
        );
        app.settings.variant = VariantSpec::Full;
        assert!(handle_key(key(KeyCode::Char('s')), &mut ui, &app).is_empty());
        assert_eq!(ui.confirm, Some(Confirm::StartFullRes));
        assert_eq!(
            handle_key(key(KeyCode::Char('y')), &mut ui, &app),
            vec![Action::Start]
        );
        assert!(ui.confirm.is_none());
    }

    #[test]
    fn settings_navigation_and_toggles() {
        let mut ui = UiState {
            focus: Focus::Settings,
            ..Default::default()
        };
        let app = ready_state();
        handle_key(key(KeyCode::Down), &mut ui, &app);
        handle_key(key(KeyCode::Down), &mut ui, &app);
        assert_eq!(ui.settings_row(), SettingsRow::Variant);
        assert_eq!(
            handle_key(key(KeyCode::Right), &mut ui, &app),
            vec![Action::SetVariant(VariantSpec::MedRes)]
        );
        assert_eq!(
            handle_key(key(KeyCode::Left), &mut ui, &app),
            vec![Action::SetVariant(VariantSpec::Full)]
        );
        ui.settings_row = 4;
        assert_eq!(
            handle_key(key(KeyCode::Enter), &mut ui, &app),
            vec![Action::SetOverwrite(true)]
        );
        ui.settings_row = 0;
        handle_key(key(KeyCode::Enter), &mut ui, &app);
        for c in "1925-1921".chars() {
            handle_key(key(KeyCode::Char(c)), &mut ui, &app);
        }
        assert_eq!(
            handle_key(key(KeyCode::Enter), &mut ui, &app),
            vec![Action::SetYears(Some((1921, 1925)))]
        );
    }

    #[test]
    fn quit_asks_while_running() {
        let mut ui = UiState::default();
        let mut app = ready_state();
        assert_eq!(
            handle_key(key(KeyCode::Char('q')), &mut ui, &app),
            vec![Action::Quit]
        );
        app.run.phase = Phase::Running;
        assert!(handle_key(key(KeyCode::Char('q')), &mut ui, &app).is_empty());
        assert_eq!(ui.confirm, Some(Confirm::QuitRunning));
        assert!(handle_key(key(KeyCode::Char('n')), &mut ui, &app).is_empty());
        assert_eq!(
            handle_key(key(KeyCode::Char('p')), &mut ui, &app),
            vec![Action::Pause]
        );
        let _ = Ark::new("x");
    }
}
