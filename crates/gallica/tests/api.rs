//! Integration tests against a local mock of the Gallica endpoints.

use std::time::Duration;

use gallica::document::PreviewSize;
use gallica::download::DownloadOptions;
use gallica::iiif::{ImageRequest, Size};
use gallica::sru::SearchQuery;
use gallica::{Ark, Client, Error, Quotas, Resource, RetryPolicy};
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn fixture(name: &str) -> String {
    std::fs::read_to_string(format!(
        "{}/tests/fixtures/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

fn xml(name: &str) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_raw(fixture(name), "text/xml; charset=UTF-8")
}

fn json(name: &str) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_raw(fixture(name), "application/json")
}

async fn client(server: &MockServer) -> Client {
    Client::builder()
        .base_url(server.uri())
        .unwrap()
        .min_request_interval(Duration::ZERO)
        .quotas(Quotas::unlimited())
        .retry(RetryPolicy {
            max_attempts: 3,
            initial_backoff: Duration::from_millis(10),
            max_backoff: Duration::from_millis(50),
        })
        .build()
        .unwrap()
}

#[tokio::test]
async fn issues_years_and_list() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/services/Issues"))
        .and(query_param("ark", "ark:/12148/cb32798952c/date"))
        .and(query_param("date", "1937"))
        .respond_with(xml("issues_1937.xml"))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/services/Issues"))
        .and(query_param("ark", "ark:/12148/cb32798952c/date"))
        .respond_with(xml("issues_years.xml"))
        .mount(&server)
        .await;
    let c = client(&server).await;
    let ark = Ark::parse("https://gallica.bnf.fr/ark:/12148/cb32798952c/date").unwrap();

    let years = c.issue_years(&ark).await.unwrap();
    assert_eq!(years.years, vec![1937, 1938]);
    assert_eq!(years.total_issues, Some(62));
    assert_eq!(years.uc3, Some(true));

    let list = c.issues(&ark, 1937).await.unwrap();
    assert_eq!(list.year, 1937);
    assert_eq!(list.issues.len(), 3);
    let first = &list.issues[0];
    assert_eq!(first.ark.name(), "bpt6k5505325m");
    assert_eq!(first.day_of_year, Some(77));
    assert_eq!(first.date().unwrap().to_string(), "1937-03-18");
    assert_eq!(first.label, "1937/03/18 (A1,N2).");

    // periodical_issues enumerates every year (1938 answers with the same body here).
    let all = c.periodical_issues(&ark, Some(1937..=1937)).await.unwrap();
    assert_eq!(all.len(), 3);
}

#[tokio::test]
async fn oai_record() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/services/OAIRecord"))
        .and(query_param("ark", "bpt6k5738219s"))
        .respond_with(xml("oairecord.xml"))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/services/OAIRecord"))
        .and(query_param("ark", "bpt6k1279113"))
        .respond_with(xml("oairecord_audio.xml"))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/services/OAIRecord"))
        .and(query_param("ark", "missing"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;
    let c = client(&server).await;

    let rec = c
        .oai_record(&Ark::new("bpt6k5738219s").unwrap())
        .await
        .unwrap();
    assert_eq!(rec.ark().unwrap().name(), "bpt6k5738219s");
    assert_eq!(
        rec.dc.title().unwrap(),
        "La plage d'Etretat par l'auteur de \"Monsieur X et Mme ***\""
    );
    assert_eq!(rec.dc.date(), Some("1868"));
    assert_eq!(rec.dc.view_count(), Some(374));
    assert_eq!(
        rec.dc.catalogue_record(),
        Some("http://catalogue.bnf.fr/ark:/12148/cb33539190h")
    );
    assert!(rec.dc.is_public_domain());
    assert_eq!(rec.typedoc.as_deref(), Some("monographies"));
    assert_eq!(rec.ocr_quality, Some(92.57));
    assert!(rec.has_full_text());
    assert_eq!(rec.set_specs.len(), 2);
    assert_eq!(rec.datestamp.as_deref(), Some("2012-01-27"));
    assert_eq!(rec.nb_issue, Some(1));
    assert_eq!(rec.sdewey.as_deref(), Some("84"));

    let audio = c
        .oai_record(&Ark::new("bpt6k1279113").unwrap())
        .await
        .unwrap();
    assert_eq!(audio.typedoc.as_deref(), Some("sonore"));
    assert_eq!(audio.sounds.len(), 2);
    assert_eq!(audio.sounds[1].page, Some(1));
    assert_eq!(
        audio.sounds[1].file,
        "https://gallica.bnf.fr/ark:/12148/bpt6k1279113/f2.audio"
    );
    assert!(!audio.has_full_text());

    let err = c
        .oai_record(&Ark::new("missing").unwrap())
        .await
        .unwrap_err();
    assert!(matches!(err, Error::NotFound(_)), "{err}");
}

#[tokio::test]
async fn pagination_and_views() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/services/Pagination"))
        .and(query_param("ark", "bpt6k5738219s"))
        .respond_with(xml("pagination.xml"))
        .mount(&server)
        .await;
    let c = client(&server).await;
    let ark = Ark::new("bpt6k5738219s").unwrap();

    let p = c.pagination(&ark).await.unwrap();
    assert_eq!(p.first_displayed_page, Some(12));
    assert!(p.has_toc);
    assert_eq!(p.toc_location, Some(328));
    assert_eq!(p.id_upn.as_deref(), Some("NUMM-5738219"));
    assert_eq!(p.view_count(), 3);
    assert_eq!(p.pages.len(), 3);
    assert!(!p.pages[0].is_numbered());
    assert!(p.pages[1].is_numbered());
    assert_eq!(
        p.pages[1].legend.as_deref(),
        Some("Le Louvre. N ° Atget : 5169.")
    );
    assert_eq!(p.pages[2].pagination_type.as_deref(), Some("R"));

    let views = c.document_views(&ark).await.unwrap();
    assert_eq!(
        views.iter().map(|v| v.view).collect::<Vec<_>>(),
        vec![1, 2, 3]
    );
    assert_eq!(views[1].label.as_deref(), Some("1"));
}

#[tokio::test]
async fn views_fall_back_to_manifest() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/services/Pagination"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/iiif/ark:/12148/bpt6k5738219s/manifest.json"))
        .respond_with(json("manifest.json"))
        .mount(&server)
        .await;
    let c = Client::builder()
        .base_url(server.uri())
        .unwrap()
        .min_request_interval(Duration::ZERO)
        .quotas(Quotas::unlimited())
        .retry(RetryPolicy::none())
        .build()
        .unwrap();
    let ark = Ark::new("bpt6k5738219s").unwrap();
    let views = c.document_views(&ark).await.unwrap();
    assert_eq!(views.len(), 2);
    assert_eq!(views[0].view, 1);
    assert_eq!(views[0].label.as_deref(), Some("NP"));
}

#[tokio::test]
async fn content_search() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/services/ContentSearch"))
        .and(query_param("ark", "bpt6k5460422k"))
        .and(query_param("query", "hugo"))
        .and(query_param("page", "173"))
        .respond_with(xml("contentsearch_page.xml"))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/services/ContentSearch"))
        .and(query_param("ark", "bpt6k5460422k"))
        .and(query_param("query", "hugo"))
        .respond_with(xml("contentsearch.xml"))
        .mount(&server)
        .await;
    let c = client(&server).await;
    let ark = Ark::new("bpt6k5460422k").unwrap();

    let r = c.content_search(&ark, "hugo", None, None).await.unwrap();
    assert_eq!(r.count, 2);
    assert_eq!(r.query, "hugo");
    assert_eq!(r.items[0].page, Some(357));
    assert_eq!(r.items[0].excerpt, "e V. HUGO");
    assert_eq!(r.items[0].highlights, vec!["HUGO"]);
    assert_eq!(r.items[1].page, Some(173));

    let r = c
        .content_search(&ark, "hugo", Some(173), None)
        .await
        .unwrap();
    assert_eq!(r.items.len(), 1);
    let item = &r.items[0];
    assert_eq!(item.page_width, Some(1153));
    assert_eq!(item.page_height, Some(2138));
    assert_eq!(item.words.len(), 1);
    assert_eq!(item.words[0].x, 514);
    assert_eq!(item.words[0].y, 915);
    assert_eq!(item.words[0].width, 101);
    assert_eq!(item.words[0].height, 47);
    assert_eq!(item.words[0].id, "PAG_00000173_ST000061");
}

#[tokio::test]
async fn sru_search_and_paging() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/SRU"))
        .and(query_param("operation", "searchRetrieve"))
        .and(query_param("version", "1.2"))
        .and(query_param("startRecord", "1"))
        .and(query_param("maximumRecords", "2"))
        .respond_with(xml("sru.xml"))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/SRU"))
        .and(query_param("startRecord", "3"))
        .respond_with(xml("sru_page2.xml"))
        .mount(&server)
        .await;
    let c = client(&server).await;

    let q = SearchQuery::new(r#"dc.creator all "molière""#).maximum_records(2);
    let page = c.search(q.clone()).await.unwrap();
    assert_eq!(page.number_of_records, 3);
    assert_eq!(page.next_record_position, Some(3));
    assert_eq!(page.records.len(), 2);
    let first = &page.records[0];
    assert_eq!(first.ark().unwrap().name(), "bpt6k5738219s");
    assert_eq!(first.title(), Some("Oeuvres complètes de Molière. Tome 1"));
    assert_eq!(first.typedoc.as_deref(), Some("monographie"));
    assert_eq!(first.ocr_quality, Some(95.5));
    assert_eq!(first.position, Some(1));
    assert_eq!(
        first.thumbnail.as_deref(),
        Some("https://gallica.bnf.fr/ark:/12148/bpt6k5738219s.thumbnail")
    );

    let all = c.search_all(q.clone(), None).await.unwrap();
    assert_eq!(all.len(), 3);
    assert_eq!(all[2].ark().unwrap().name(), "bpt6k1234567z");

    let limited = c.search_all(q, Some(1)).await.unwrap();
    assert_eq!(limited.len(), 1);
}

#[tokio::test]
async fn iiif_info_manifest_and_image() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/iiif/ark:/12148/bpt6k5738219s/f1/info.json"))
        .respond_with(json("info.json"))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/iiif/ark:/12148/bpt6k5738219s/manifest.json"))
        .respond_with(json("manifest.json"))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(
            "/iiif/ark:/12148/bpt6k5738219s/f1/full/full/0/native.jpg",
        ))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw(b"\xFF\xD8JPEGDATA".to_vec(), "image/jpeg"),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(
            "/iiif/ark:/12148/bpt6k5738219s/f2/full/1000,/0/native.jpg",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_raw("<html>blocked</html>", "text/html"))
        .mount(&server)
        .await;
    let c = client(&server).await;
    let ark = Ark::new("bpt6k5738219s").unwrap();

    let info = c.iiif_info(&ark, 1).await.unwrap();
    assert_eq!((info.width, info.height), (2385, 3700));
    assert_eq!(info.sizes.len(), 2);
    assert_eq!(info.tiles[0].scale_factors, vec![1, 2, 4, 8, 16]);

    let m = c.iiif_manifest(&ark).await.unwrap();
    assert_eq!(m.label.as_deref(), Some("La plage d'Etretat"));
    assert_eq!(m.canvases.len(), 2);
    assert_eq!(
        m.canvases[0].service_id.as_deref(),
        Some("https://gallica.bnf.fr/iiif/ark:/12148/bpt6k5738219s/f1")
    );
    assert_eq!(m.canvases[1].view, 2);
    assert!(m.canvases[1].service_id.is_none());

    let bytes = c.iiif_image(&ark, 1, &ImageRequest::full()).await.unwrap();
    assert_eq!(&bytes[..2], b"\xFF\xD8");

    let err = c
        .iiif_image(&ark, 2, &ImageRequest::full().size(Size::Width(1000)))
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Http { .. }), "{err}");
}

#[tokio::test]
async fn download_with_retry_and_skip() {
    let server = MockServer::start().await;
    // First call fails with 503, second succeeds.
    Mock::given(method("GET"))
        .and(path(
            "/iiif/ark:/12148/bpt6k5738219s/f1/full/full/0/native.jpg",
        ))
        .respond_with(ResponseTemplate::new(503))
        .up_to_n_times(1)
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(
            "/iiif/ark:/12148/bpt6k5738219s/f1/full/full/0/native.jpg",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_raw(vec![7u8; 5000], "image/jpeg"))
        .expect(1)
        .mount(&server)
        .await;
    let c = client(&server).await;
    let ark = Ark::new("bpt6k5738219s").unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("nested/dir/f1.jpg");

    let seen = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let seen2 = seen.clone();
    let opts = DownloadOptions::new().progress(move |done, total| {
        assert_eq!(total, Some(5000));
        seen2.store(done, std::sync::atomic::Ordering::SeqCst);
    });
    let out = c
        .download_iiif_image(&ark, 1, &ImageRequest::full(), &path, &opts)
        .await
        .unwrap();
    assert!(!out.skipped);
    assert_eq!(out.bytes, 5000);
    assert_eq!(out.content_type.as_deref(), Some("image/jpeg"));
    assert_eq!(std::fs::read(&path).unwrap().len(), 5000);
    assert_eq!(seen.load(std::sync::atomic::Ordering::SeqCst), 5000);
    assert!(!dir.path().join("nested/dir/f1.jpg.part").exists());

    // Second download is skipped because the file exists.
    let out = c
        .download_iiif_image(
            &ark,
            1,
            &ImageRequest::full(),
            &path,
            &DownloadOptions::new(),
        )
        .await
        .unwrap();
    assert!(out.skipped);
    assert_eq!(out.bytes, 5000);
}

#[tokio::test]
async fn retries_give_up() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/services/Pagination"))
        .respond_with(ResponseTemplate::new(429).insert_header("Retry-After", "0"))
        .expect(3)
        .mount(&server)
        .await;
    let c = client(&server).await;
    let err = c
        .pagination(&Ark::new("bpt6k5738219s").unwrap())
        .await
        .unwrap_err();
    match err {
        Error::RetriesExhausted { attempts, last, .. } => {
            assert_eq!(attempts, 3);
            assert_eq!(last.status(), Some(429));
        }
        other => panic!("unexpected error {other}"),
    }
}

#[tokio::test]
async fn text_alto_pdf_and_preview_urls() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/ark:/12148/bpt6k5619759j/f5n2.texteBrut"))
        .respond_with(ResponseTemplate::new(200).set_body_string("<html>texte</html>"))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/RequestDigitalElement"))
        .and(query_param("O", "bpt6k5619759j"))
        .and(query_param("E", "ALTO"))
        .and(query_param("Deb", "3"))
        .respond_with(ResponseTemplate::new(200).set_body_string("<alto/>"))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/ark:/12148/bpt6k5619759j/f2.thumbnail"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(b"img".to_vec(), "image/jpeg"))
        .mount(&server)
        .await;
    let c = client(&server).await;
    let ark = Ark::new("bpt6k5619759j").unwrap();

    assert_eq!(
        c.text(&ark, Some((5, 2))).await.unwrap(),
        "<html>texte</html>"
    );
    assert_eq!(c.alto(&ark, 3).await.unwrap(), "<alto/>");
    assert_eq!(
        &c.preview(&ark, 2, PreviewSize::Thumbnail).await.unwrap()[..],
        b"img"
    );
    assert!(
        c.pdf_url(&ark, None)
            .unwrap()
            .as_str()
            .ends_with("/ark:/12148/bpt6k5619759j.pdf")
    );
    assert!(
        c.pdf_url(&ark, Some((1, 10)))
            .unwrap()
            .as_str()
            .ends_with("/ark:/12148/bpt6k5619759j/f1n10.pdf")
    );
    assert!(
        c.text_url(&ark, None)
            .unwrap()
            .as_str()
            .ends_with("/ark:/12148/bpt6k5619759j.texteBrut")
    );
}

#[test]
fn resource_parsing() {
    let r = Resource::parse("https://gallica.bnf.fr/ark:/12148/cb327110547/").unwrap();
    assert!(matches!(r, Resource::Periodical { .. }));
    let r = Resource::parse("https://gallica.bnf.fr/ark:/12148/bpt6k961287k").unwrap();
    assert!(matches!(r, Resource::Document { view: None, .. }));
}

#[tokio::test]
async fn download_view_with_variants() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/ark:/12148/bpt6k961287k/f2.highres"))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw(b"\xFF\xD8highres".to_vec(), "image/jpeg"),
        )
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(
            "/iiif/ark:/12148/bpt6k961287k/f2/full/full/0/native.jpg",
        ))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw(b"\xFF\xD8full".to_vec(), "image/jpeg"),
        )
        .expect(1)
        .mount(&server)
        .await;
    let c = client(&server).await;
    let ark = Ark::new("bpt6k961287k").unwrap();
    let dir = tempfile::tempdir().unwrap();

    let variant = gallica::ImageVariant::default();
    let path = dir.path().join(variant.file_name(2));
    let out = c
        .download_view(&ark, 2, &variant, &path, &DownloadOptions::new())
        .await
        .unwrap();
    assert_eq!(out.bytes, 9);
    assert_eq!(
        std::fs::read(dir.path().join("f0002.jpg")).unwrap(),
        b"\xFF\xD8highres"
    );

    let bytes = c
        .view_image(&ark, 2, &gallica::ImageVariant::full())
        .await
        .unwrap();
    assert_eq!(&bytes[..], b"\xFF\xD8full");
}
