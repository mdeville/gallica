# gallica — application specification

Status: implemented 2026-10-26 (core, headless commands, TUI, GUI); kept as the design reference. Covers the terminal UI (ratatui), the
desktop GUI (egui/eframe) and the shared application core they are built on.
The SDK (`crates/gallica`) is specified by its own API documentation.

## 1. Goal

Give researchers a tool that downloads Gallica collections automatically and
politely: paste a Gallica URL or ARK, see what it is and what a download would
cost, choose a time range and an image size, start, watch progress, stop and
resume later. The same workflow is offered in three ways:

| Mode | Binary | For |
|---|---|---|
| Headless commands | `gallica info|plan|download` | scripts, servers, our own tests |
| Terminal UI | `gallica tui` (default when run with no subcommand in a terminal) | SSH sessions, keyboard users |
| Desktop GUI | `gallica-gui` | everyone else |

## 2. Framework choice

- **Terminal:** `ratatui` + `crossterm` (both on blessed.rs).
- **Desktop:** `egui` through `eframe` (blessed.rs, "immediate-mode UI with lots
  of widgets"). It is chosen over `iced` because it shares ratatui's
  immediate-mode model: a panel is a function that receives the current state,
  draws it and returns the user's actions, called once per frame. The two
  front ends therefore have the same shape, panel for panel, and the
  non-drawing code is shared outright. `eframe` runs on Linux, macOS and
  Windows (and the web, which we do not target) through `winit`.
  `egui_extras` provides image loading for page previews.
- **Headless:** `clap` for arguments, `indicatif` for progress bars.
- **Shared:** `tokio` runtime, `config` to load
  settings (file and environment) and `toml` to save them, `serde_json` for
  job files, `directories` for platform config paths, `tracing` for every
  log line (see 4.7), `tokio-util` for cancellation tokens.

What is shared is not widgets, which cannot be, but everything that decides
what a widget shows: the state, the actions, the engine, the formatting.

## 3. Crate layout

```
crates/
  gallica/        SDK (exists)
  gallica-core/   application core, UI-agnostic, fully testable with a mock server
  gallica-tui/    binary `gallica`: clap subcommands + ratatui front end
  gallica-gui/    binary `gallica-gui`: eframe front end
```

`gallica-core` depends on `gallica` only. Both front ends depend on
`gallica-core` and never call the SDK directly.

## 4. Application core (`gallica-core`)

The core follows the Elm shape that both ratatui and egui apps naturally take:

```
            Action (user intent)                 Event (engine report)
Front end ───────────────────▶ App ◀────────────────────────── Engine
          ◀─────────────────── │ ───────────────────────────▶
            &AppState to draw  │  Command (SDK work to perform)
```

- `AppState`: everything a front end needs to draw. Plain data, `Clone`,
  no SDK types that cannot be displayed.
- `Action`: what the user can do. One enum, identical for all front ends.
- `Event`: what the engine reports back, through an unbounded `mpsc` channel.
- `App::update(&mut self, Action | Event) -> Vec<Command>`: pure state
  transition, unit tested without any I/O.
- `Engine`: owns the `gallica::Client` and the tokio runtime handle, executes
  `Command`s as tasks, emits `Event`s. Supports pause (a `watch` gate checked
  between views) and cancel (a `CancellationToken`).

### 4.1 State

```rust
pub struct AppState {
    pub target: Target,            // input text, resolution status, resolved resource
    pub settings: Settings,        // see 4.3
    pub plan: Option<Plan>,        // what a run would download, with estimates
    pub run: Run,                  // phase, per-item status, progress counters
}

pub enum Resolved {
    Periodical { ark, title, years: Vec<u16>, total_issues: Option<u32> },
    Document   { ark, title, date, typedoc, views: u32, rights: Vec<String> },
}

pub struct Plan {
    pub items: Vec<PlanItem>,      // one per issue (periodical) or one (document)
    pub views: u64,                // total views, when known; else an estimate
    pub estimate: Estimate,        // requests per quota class, minimum duration, bytes guess
}

pub struct PlanItem { pub id: ItemId, pub ark: Ark, pub date: Option<Date>, pub label: String, pub views: Option<u32> }

pub enum Phase { Idle, Resolving, Planning, Ready, Running, Paused, Finished, Failed(String) }

pub struct ItemState { pub status: ItemStatus, pub done_views: u32, pub total_views: Option<u32>, pub bytes: u64, pub error: Option<String> }
pub enum ItemStatus { Pending, Fetching, Done, Skipped, Failed }

pub struct Progress { pub items_done, items_total, views_done, views_total, bytes, started_at, rate_bps, quota_waits, eta }
```

### 4.2 Actions and events

```rust
pub enum Action {
    SetInput(String), Resolve,
    SetYears(Option<RangeInclusive<u16>>), SetMaxIssues(Option<usize>),
    SetVariant(ImageVariant), SetOutputDir(PathBuf), SetOverwrite(bool), SetSaveText(bool),
    BuildPlan, Start, Pause, Resume, Cancel, RetryFailed,
    OpenJob(PathBuf), Quit,
}

pub enum Event {
    Resolved(Result<Resolved, String>),
    Planned(Result<Plan, String>),
    ItemStarted(ItemId), ViewDone { item: ItemId, view: u32, bytes: u64, skipped: bool },
    ItemFinished(ItemId, Result<(), String>),
    QuotaWait { class: QuotaClass, for_: Duration },
    Log(LogLevel, String),
    RunFinished { failed: u32 },
}
```

### 4.3 Settings

Persisted as TOML in the platform config directory
(`directories::ProjectDirs::from("", "", "gallica")`, e.g.
`~/.config/gallica/config.toml`). Loading goes through the `config` crate,
layering, last wins: the defaults, the file (optional), then `GALLICA_*`
environment variables (`GALLICA_CONTACT`, `GALLICA_OUTPUT_DIR`, and `__` for
nested keys: `GALLICA_QUOTAS__TEXTS_PER_MINUTE`). The UI and CLI flags then
override per run. Saving writes the TOML file, including any values that came
from the environment.

```toml
contact = "you@example.org"          # appended to the User-Agent; required before a run
output_dir = "~/Gallica"
variant = "highres"                   # highres | medres | width:<px> | full
overwrite = false
save_text = false                     # also fetch texteBrut per issue (5/min quota)
[quotas]                              # optional overrides if the BnF grants more
full_res_images_per_minute = 5
```

### 4.4 Planning

1. Resolve the input with `Resource::parse` and fetch what describes it:
   `issue_years` for a periodical, `oai_record` + `pagination` for a document.
2. Build the plan: for a periodical, one `Issues` call per selected year,
   producing one item per issue (`max_issues` truncates, for test runs). View
   counts are unknown until each issue's `pagination` is fetched during the
   run, so the estimate uses the known average (8 views per issue for the
   periodical sampled) until real numbers replace it.
3. Estimate: requests per quota class (metadata calls are `Default`, images
   follow the variant's class) and `Quotas::minimum_duration`. The UI shows it
   before Start and asks for confirmation when the variant is full resolution.

### 4.5 Run

Items are processed sequentially (the quotas make parallel issues pointless);
views inside an item are fetched with the SDK's bounded concurrency. Per item:

1. `oai_record` → write `metadata.json` (Dublin Core + typedoc + OCR quality).
2. `pagination` → view list with master sizes → update totals.
3. For each view, `download_view` with the chosen variant. Existing files are
   skipped by the SDK, which is what makes a re-run a resume.
4. Optionally `text` → `text.html`.
5. Write `done.json` (variant, view count, timestamp). On a later run an item
   with a matching `done.json` is skipped without any request.

Errors on one view are retried by the SDK; a persistent failure marks the item
`Failed` with the message, and the run continues. A Cloudflare challenge (HTTP
403 with the "Just a moment" body) is recognised and reported as one clear log
line, and the run stops, since every further request would fail the same way.

### 4.6 Output layout

```
<output_dir>/
  cb327110547 - Les Belles images/        periodical: "<ark> - <title>"
    periodical.json                        years, total issues, DC record
    1921/
      1921-11-03 bpt6k961263c/            "<date> <ark>", date from dayOfYear
        metadata.json
        f0001.jpg … f0008.jpg
        text.html                          only with save_text
        done.json
  bpt6k961287k - Les Belles images/       single document: same inner layout
```

Titles are sanitised for file systems (no `/ \ : * ? " < > |`, trimmed, at
most 80 characters). A `job.json` at the root of each target directory stores
the plan and settings of the last run so `OpenJob` can restore it.

### 4.7 Logging

Everything is logged through `tracing`. `App::log` emits the messages meant
for the user (resolution, plan, failures, run summary) with the `gallica_app`
target; the SDK and the engine log diagnostics under their module targets,
and `log` crate records from dependencies are bridged in. `gallica_core::log`
holds the pieces each front end assembles:

- one filter for every output, from `Verbosity` unless `RUST_LOG` is set:
  normal (app messages at info, the rest at warn), `-v` (info, debug from
  gallica) and `--debug` (debug from everything, dependencies included);
- one line format, `level text`, where diagnostics prefix the text with
  their target;
- outputs: stderr, a `LogBuffer` (the last 500 entries) that the log panels
  show, and a log file.

| Front end | Outputs |
|-----------|---------|
| headless commands | stderr, printed above the progress bars |
| terminal UI | log panel, `gallica.log` |
| desktop GUI | log panel, stderr, `gallica-gui.log` |

Log files go to the platform state directory (`~/.local/state/gallica` on
Linux; the local data directory elsewhere) and are truncated at start; their
lines are prefixed with an RFC 3339 timestamp.

## 5. Panels

Both front ends show the same six panels, fed from the same `AppState`:

| Panel | Shows | Actions |
|---|---|---|
| Target | input field, Resolve, resolution status, resolved title/type | SetInput, Resolve |
| Details | years available, issue/view counts, rights, the plan estimate (requests, minimum time, size guess) | — |
| Settings | year range, max issues, variant, output dir, overwrite, save text, contact | Set* |
| Queue | table: date, ark, label, views done/total, status, per-item bar; selection shows the item's error | RetryFailed |
| Progress | overall bar, items and views done, bytes, rate, ETA, quota waits, phase | Start, Pause, Resume, Cancel |
| Log | last N log lines with level (see 4.7) | clear (front end only) |

### 5.1 Terminal layout (minimum 80×24)

```
┌ Target ────────────────────────────────────────────────────────────────────┐
│ > https://gallica.bnf.fr/ark:/12148/cb327110547/      Les Belles images ✓  │
├ Details ──────────────────────────┬ Queue ─────────────────────────────────┤
│ Periodical, 1905–1936, 1186 issues│ date        ark           views  state │
│ Plan: 24 issues, ~192 views       │ 1921-11-03  bpt6k961263c   8/8   done  │
│ highres · 50/min · ≥ 4 min        │ 1921-11-10  bpt6k961264r   3/8   ▇▇▇░░ │
├ Settings ─────────────────────────┤ …                                      │
│ Years  [1921]–[1922]  Max issues []│                                        │
│ Image  (•) highres ( ) width 1000  │                                        │
│        ( ) full                    │                                        │
│ Output ~/Gallica   Overwrite [ ]   │                                        │
├ Progress ──────────────────────────┴────────────────────────────────────────┤
│ Running  ▇▇▇▇▇▇░░░░░░░░ 41%  10/24 issues  83/192 views  1.1 MB/s  ETA 3m │
├ Log ───────────────────────────────────────────────────────────────────────┤
│ 12:01:03 info  bpt6k961264r: 8 views                                       │
│ 12:01:09 warn  quota: waiting 12 s (highres)                               │
└ Tab focus · Enter resolve/edit · s start · p pause · c cancel · ? help · q ┘
```

Keys: `Tab`/`Shift-Tab` move focus between panels, `Enter` activates (resolve
in Target, edit in Settings, show details in Queue), `↑↓` navigate, `s` start,
`p` pause/resume, `c` cancel (confirmation), `r` retry failed, `?` help
overlay, `q` quit (confirmation while running). Full-resolution runs open a
confirmation dialog with the time estimate.

### 5.2 Desktop layout (eframe)

- `TopBottomPanel::top`: Target (text field, Resolve button, status).
- `SidePanel::left` (resizable): Details above Settings; a native folder
  picker for the output directory (`rfd`, see open questions).
- `CentralPanel`: Queue as a `TableBuilder` with a selectable row; the selected
  item shows its views as thumbnails when `done`.
- `TopBottomPanel::bottom`: Progress with buttons, and a collapsible Log.
- Menu: File (Open job, Settings file location, Quit), Help (About, the BnF
  quotas page).

Repaints are requested whenever an `Event` arrives, so the UI stays idle
between events.

## 6. Headless commands

```
gallica info <target>                          resolve and print what it is
gallica plan <target> [--years A-B] [--max-issues N] [--variant V]
                                               print the plan and the estimate
gallica download <target> --out DIR [same flags] [--yes] [--save-text]
                                               run with indicatif progress bars
gallica tui [<target>]                         the terminal UI
```

All subcommands read `config.toml` and the `GALLICA_*` variables; `--contact`
is required if neither sets one. `download` refuses full resolution without `--yes` after printing the
estimate. Exit codes: 0 success, 1 some items failed, 2 bad arguments,
3 blocked by Gallica (challenge or 403). `-v` and `--debug` raise the log
level (see 4.7).

## 7. Testing

- `gallica-core`: `App::update` unit tests (pure); engine tests against a
  wiremock Gallica that serves the real fixtures, checking the output layout,
  resume (second run makes no image request), pause/cancel, failure handling.
- `gallica-tui`: render tests with ratatui's `TestBackend` on fixed states
  (snapshot the buffer as text); key handling tests mapping keys to actions.
- `gallica-gui`: compiled and clippy-checked in CI; behaviour lives in the core.
  Manual testing by Matthew, since this environment has no display.
- Live runs: by Matthew, once the BnF answers about automated access.

## 8. Implementation order

1. `gallica-core`: state, actions, `update`, plan/estimate, output layout, job
   file, engine. Tests with the mock server.
2. `gallica` binary, headless subcommands. Usable end to end from a script.
3. `gallica tui`.
4. `gallica-gui`.

## 9. Open questions

1. Folder naming: "`<ark> - <title>`" as above, or title first? ARK first keeps
   folders unique and sortable; title first reads better.
2. Native file dialogs in the GUI need `rfd` (not on blessed.rs, widely used).
   Alternative: a plain text field. Proposal: `rfd`.
3. Should `save_text` also fetch ALTO per view? Deferred; `texteBrut` per issue
   is enough for a first version.
4. GUI binary name: `gallica-gui`, or a single `gallica` binary that opens the
   GUI when launched outside a terminal? Separate binaries keep the GUI's
   dependencies out of the server build. Proposal: separate.
