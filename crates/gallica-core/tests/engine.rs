//! End to end: App + Engine against a mock Gallica serving the real fixtures.

use std::path::Path;
use std::time::Duration;

use gallica::Quotas;
use gallica_core::{
    Action, App, Engine, Event, ItemStatus, LogBuffer, LogEntry, Phase, Settings, VariantSpec,
};
use wiremock::matchers::{method, path, path_regex, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn fixture(name: &str) -> String {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../gallica/tests/fixtures/real");
    std::fs::read_to_string(root.join(name)).unwrap()
}

fn xml(name: &str) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_raw(fixture(name), "text/xml; charset=UTF-8")
}

async fn mock_gallica() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/services/Issues"))
        .and(query_param("date", "1921"))
        .respond_with(xml("issues_1921.xml"))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/services/Issues"))
        .and(query_param("date", "1905"))
        .respond_with(xml("issues_1921.xml"))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/services/Issues"))
        .respond_with(xml("issues_years.xml"))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/services/OAIRecord"))
        .respond_with(xml("oairecord.xml"))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/services/Pagination"))
        .respond_with(xml("pagination.xml"))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path_regex(
            r"^/ark:/12148/bpt6k96126[34][cr]/f[1-8]\.highres$",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_raw(vec![0xFFu8; 1000], "image/jpeg"))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path_regex(r"\.texteBrut$"))
        .respond_with(ResponseTemplate::new(200).set_body_string("<html>texte</html>"))
        .mount(&server)
        .await;
    server
}

fn settings(server: &MockServer, out: &Path) -> Settings {
    Settings {
        contact: Some("test@example.org".into()),
        output_dir: out.to_path_buf(),
        variant: VariantSpec::HighRes,
        years: Some((1921, 1921)),
        quotas: Some(Quotas::unlimited()),
        min_request_interval_ms: Some(0),
        base_url: Some(server.uri()),
        ..Settings::default()
    }
}

struct Harness {
    app: App,
    engine: Engine,
    rx: gallica_core::engine::EventReceiver,
    log: LogBuffer,
    _log_guard: gallica_core::log::CaptureGuard,
}

impl Harness {
    fn new(settings: Settings) -> Harness {
        let (log, _log_guard) = gallica_core::log::capture();
        let (tx, rx) = gallica_core::engine::channel();
        let engine = Engine::new(&settings, tx, tokio::runtime::Handle::current()).unwrap();
        Harness {
            app: App::new(settings),
            engine,
            rx,
            log,
            _log_guard,
        }
    }

    /// The application messages logged so far.
    fn messages(&self) -> Vec<LogEntry> {
        let mut entries = self.log.snapshot();
        entries.retain(|e| e.target.is_none());
        entries
    }

    fn act(&mut self, action: Action) {
        for cmd in self.app.update(action) {
            // Settings are not written to the real config dir in tests.
            if !matches!(cmd, gallica_core::Command::SaveSettings(_)) {
                self.engine.execute(cmd);
            }
        }
    }

    /// Pumps events until `done` holds or the timeout elapses.
    async fn wait_until(&mut self, done: impl Fn(&App) -> bool) {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
        while !done(&self.app) {
            let event = tokio::time::timeout_at(deadline, self.rx.recv())
                .await
                .unwrap_or_else(|_| panic!("timeout; log: {:#?}", self.log.snapshot()))
                .expect("engine dropped");
            let cancelled = matches!(
                event,
                Event::RunFinished {
                    cancelled: true,
                    ..
                }
            );
            for cmd in self.app.apply(event) {
                if !matches!(cmd, gallica_core::Command::SaveSettings(_)) {
                    self.engine.execute(cmd);
                }
            }
            let _ = cancelled;
        }
    }
}

#[tokio::test]
async fn downloads_a_periodical_and_resumes() {
    let server = mock_gallica().await;
    let out = tempfile::tempdir().unwrap();
    let mut h = Harness::new(settings(&server, out.path()));

    h.act(Action::SetInput(
        "https://gallica.bnf.fr/ark:/12148/cb327110547/".into(),
    ));
    h.act(Action::Resolve);
    h.wait_until(|a| a.state.run.phase == Phase::Ready).await;

    let resolved = h.app.state.target.resolved.clone().unwrap();
    assert_eq!(resolved.title(), Some("Les Belles images"));
    let plan = h.app.state.plan.clone().unwrap();
    assert_eq!(plan.items.len(), 2);
    assert_eq!(plan.items[0].date.unwrap().to_string(), "1921-11-03");
    assert_eq!(plan.estimate.views, 16);

    h.act(Action::Start);
    h.wait_until(|a| a.state.run.phase == Phase::Finished).await;
    let run = &h.app.state.run;
    assert_eq!(run.progress.failed_items, 0, "{:#?}", h.log.snapshot());
    assert!(run.items.iter().all(|i| i.status == ItemStatus::Done));
    assert_eq!(run.progress.views_done, 16);
    assert_eq!(run.progress.views_total, Some(16));
    assert_eq!(run.progress.bytes, 16_000);

    let target = out.path().join("cb327110547 - Les Belles images");
    assert!(target.join("periodical.json").is_file());
    assert!(target.join("job.json").is_file());
    let issue = target.join("1921").join("1921-11-03 bpt6k961263c");
    for v in 1..=8 {
        assert!(issue.join(format!("f{v:04}.jpg")).is_file(), "f{v:04}.jpg");
    }
    assert!(issue.join("metadata.json").is_file());
    assert!(issue.join("done.json").is_file());
    assert!(!issue.join("text.html").exists());
    let meta: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(issue.join("metadata.json")).unwrap())
            .unwrap();
    assert_eq!(meta["record"]["dc"]["titles"][0], "Les Belles images");
    assert_eq!(meta["pagination"]["pages"].as_array().unwrap().len(), 8);

    // A second run makes no image request: both items are skipped.
    h.act(Action::Start);
    h.wait_until(|a| a.state.run.phase == Phase::Finished).await;
    let run = &h.app.state.run;
    assert!(
        run.items.iter().all(|i| i.status == ItemStatus::Skipped),
        "{:?}",
        run.items
    );
    assert_eq!(run.progress.views_done, 16);
    assert_eq!(run.progress.bytes, 0);

    // Opening the job file restores the plan.
    let mut fresh = Harness::new(settings(&server, out.path()));
    fresh.act(Action::OpenJob(target.join("job.json")));
    fresh
        .wait_until(|a| a.state.run.phase == Phase::Ready)
        .await;
    assert_eq!(fresh.app.state.plan.as_ref().unwrap().items.len(), 2);
    assert_eq!(fresh.app.state.target.resolved, Some(resolved));

    // Exactly one request per view over the two runs.
    let image_requests = server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| r.url.path().ends_with(".highres"))
        .count();
    assert_eq!(image_requests, 16);
}

#[tokio::test]
async fn downloads_a_document_with_text() {
    let server = mock_gallica().await;
    let out = tempfile::tempdir().unwrap();
    let mut s = settings(&server, out.path());
    s.save_text = true;
    s.variant = VariantSpec::Width(500);
    Mock::given(method("GET"))
        .and(path_regex(
            r"^/iiif/ark:/12148/bpt6k961263c/f[1-8]/full/500,/0/native\.jpg$",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_raw(vec![1u8; 10], "image/jpeg"))
        .expect(8)
        .mount(&server)
        .await;
    let mut h = Harness::new(s);
    h.act(Action::SetInput("bpt6k961263c".into()));
    h.act(Action::Resolve);
    h.wait_until(|a| a.state.run.phase == Phase::Ready).await;
    assert!(
        !h.app
            .state
            .target
            .resolved
            .as_ref()
            .unwrap()
            .is_periodical()
    );
    assert!(h.app.state.plan.as_ref().unwrap().estimate.views_known);

    h.act(Action::Start);
    h.wait_until(|a| a.state.run.phase == Phase::Finished).await;
    assert_eq!(
        h.app.state.run.items[0].status,
        ItemStatus::Done,
        "{:#?}",
        h.log.snapshot()
    );
    let dir = out.path().join("bpt6k961263c - Les Belles images");
    assert!(dir.join("f0001.jpg").is_file());
    assert_eq!(
        std::fs::read_to_string(dir.join("text.html")).unwrap(),
        "<html>texte</html>"
    );
    assert!(dir.join("done.json").is_file());
}

#[tokio::test]
async fn a_challenge_stops_the_run() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/services/OAIRecord"))
        .respond_with(
            ResponseTemplate::new(403).set_body_raw("<html>Just a moment...</html>", "text/html"),
        )
        .mount(&server)
        .await;
    let out = tempfile::tempdir().unwrap();
    let mut h = Harness::new(settings(&server, out.path()));
    h.act(Action::SetInput("bpt6k961263c".into()));
    h.act(Action::Resolve);
    h.wait_until(|a| matches!(a.state.target.status, gallica_core::TargetStatus::Failed(_)))
        .await;
    let msg = h.messages().last().unwrap().message.clone();
    assert!(msg.contains("403"), "{msg}");
}

#[tokio::test]
async fn cancel_stops_between_items() {
    let server = mock_gallica().await;
    let out = tempfile::tempdir().unwrap();
    let mut h = Harness::new(settings(&server, out.path()));
    h.act(Action::SetInput("cb327110547".into()));
    h.act(Action::Resolve);
    h.wait_until(|a| a.state.run.phase == Phase::Ready).await;
    h.act(Action::Start);
    h.act(Action::Pause);
    h.act(Action::Cancel);
    h.wait_until(|a| !a.state.run.phase.is_active()).await;
    assert_eq!(h.app.state.run.phase, Phase::Ready);
    assert!(
        h.app
            .state
            .run
            .items
            .iter()
            .any(|i| i.status != ItemStatus::Done)
    );
}
