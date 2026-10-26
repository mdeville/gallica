//! Runs the headless commands in-process against a mock Gallica.

use std::path::Path;
use std::process::ExitCode;

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
        .and(path_regex(r"\.highres$"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(vec![0xFFu8; 100], "image/jpeg"))
        .mount(&server)
        .await;
    server
}

fn run<S: Into<std::ffi::OsString> + Clone>(args: &[S]) -> ExitCode {
    let mut full: Vec<std::ffi::OsString> = vec!["gallica".into()];
    full.extend(args.iter().cloned().map(Into::into));
    gallica_tui::main_with(full)
}

#[test]
fn info_plan_and_download() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let server = rt.block_on(mock_gallica());
    let out = tempfile::tempdir().unwrap();
    let config = out.path().join("config.toml");
    let base = server.uri();
    let common = [
        "--config",
        config.to_str().unwrap(),
        "--base-url",
        base.as_str(),
        "--no-quotas",
        "--min-interval-ms",
        "0",
        "--contact",
        "test@example.org",
        "--out",
        out.path().to_str().unwrap(),
    ];
    let with = |extra: &[&str]| -> Vec<String> {
        extra
            .iter()
            .chain(common.iter())
            .map(|s| s.to_string())
            .collect()
    };

    assert_eq!(run(&with(&["info", "cb327110547"])), ExitCode::SUCCESS);
    assert_eq!(
        run(&with(&["plan", "cb327110547", "--years", "1921"])),
        ExitCode::SUCCESS
    );

    // Full resolution needs --yes.
    assert_eq!(
        run(&with(&[
            "download",
            "cb327110547",
            "--years",
            "1921",
            "--variant",
            "full"
        ])),
        ExitCode::from(gallica_tui::EXIT_USAGE)
    );

    assert_eq!(
        run(&with(&["download", "cb327110547", "--years", "1921"])),
        ExitCode::SUCCESS
    );
    let issue = out
        .path()
        .join("cb327110547 - Les Belles images/1921/1921-11-03 bpt6k961263c");
    assert!(issue.join("f0008.jpg").is_file());
    assert!(issue.join("done.json").is_file());
    // Settings were saved to the given config file, not the platform one.
    assert!(config.is_file());

    // Bad arguments.
    assert_eq!(
        run(&with(&["download", "cb327110547", "--variant", "huge"])),
        ExitCode::from(gallica_tui::EXIT_USAGE)
    );
    assert_eq!(run(&["--version"]), ExitCode::SUCCESS);
}

#[test]
fn blocked_exit_code() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let server = rt.block_on(async {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(
                ResponseTemplate::new(403)
                    .set_body_raw("<html>Just a moment...</html>", "text/html"),
            )
            .mount(&server)
            .await;
        server
    });
    let out = tempfile::tempdir().unwrap();
    let config = out.path().join("config.toml");
    let base = server.uri();
    let code = run(&[
        "info",
        "bpt6k961263c",
        "--config",
        config.to_str().unwrap(),
        "--base-url",
        base.as_str(),
        "--no-quotas",
        "--min-interval-ms",
        "0",
    ]);
    assert_eq!(code, ExitCode::from(gallica_tui::EXIT_BLOCKED));
}
