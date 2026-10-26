//! Parser checks against answers saved from the live service on 2026-10-26
//! (periodical "Les Belles images", ark:/12148/cb327110547, and its issue
//! bpt6k961287k). These complement the documentation-based fixtures.

use gallica::document::{self, TocFormat};
use gallica::iiif::{ImageInfo, Manifest};
use gallica::sru;

fn fixture(name: &str) -> String {
    std::fs::read_to_string(format!(
        "{}/tests/fixtures/real/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

#[test]
fn issues_use_camel_case_attributes() {
    let years = document::parse_issue_years(&fixture("issues_years.xml")).unwrap();
    assert_eq!(years.parent_ark, "cb327110547/date");
    assert_eq!(years.total_issues, Some(1186));
    assert_eq!(years.uc3, Some(false));
    assert_eq!(years.years.first(), Some(&1905));
    assert_eq!(years.years.last(), Some(&1936));
    assert!(!years.years.contains(&1916), "1916 has no issue online");

    let list = document::parse_issue_list(&fixture("issues_1921.xml"), 1921).unwrap();
    assert_eq!(list.parent_ark, "cb327110547/date");
    assert_eq!(list.issues.len(), 2);
    assert_eq!(list.issues[0].ark.name(), "bpt6k961263c");
    assert_eq!(list.issues[0].label, "03 novembre 1921");
    assert_eq!(list.issues[0].date().unwrap().to_string(), "1921-11-03");
    assert_eq!(list.issues[1].date().unwrap().to_string(), "1921-11-10");
}

#[test]
fn oai_record_of_an_issue() {
    let rec = document::parse_oai_record(&fixture("oairecord.xml")).unwrap();
    assert_eq!(rec.ark().unwrap().name(), "bpt6k961287k");
    assert_eq!(rec.dc.title(), Some("Les Belles images"));
    assert_eq!(rec.dc.date(), Some("1923-08-16"));
    assert_eq!(rec.dc.view_count(), Some(11090));
    assert_eq!(rec.dc.periodical_ark().unwrap().name(), "cb327110547");
    assert_eq!(
        rec.dc.catalogue_record(),
        Some("http://catalogue.bnf.fr/ark:/12148/cb327110547")
    );
    assert_eq!(rec.typedoc.as_deref(), Some("fascicule"));
    assert_eq!(rec.ocr_quality, Some(87.43));
    assert!(rec.has_full_text());
    assert_eq!(
        rec.set_specs,
        vec!["gallica:typedoc:periodiques:fascicules"]
    );
    assert_eq!(rec.first_indexation_date.as_deref(), Some("12/03/2013"));
    assert_eq!(rec.visibility_rights.as_deref(), Some("all"));
}

#[test]
fn pagination_carries_image_sizes() {
    let p = document::parse_pagination(&fixture("pagination.xml")).unwrap();
    assert_eq!(p.view_count(), 8);
    assert!(!p.has_toc);
    assert_eq!(p.toc_location, None, "TocLocation 0 means none");
    assert_eq!(p.first_displayed_page, Some(1));
    assert_eq!(p.pages.len(), 8);
    assert_eq!(
        (p.pages[0].width, p.pages[0].height),
        (Some(4318), Some(6085))
    );
    assert_eq!(p.pages[1].label.as_deref(), Some("2"));
    assert!(p.pages[1].is_numbered());
}

#[test]
fn sru_periodical_search() {
    let r = sru::parse_search_results(&fixture("sru.xml")).unwrap();
    assert_eq!(r.number_of_records, 557);
    assert_eq!(r.records.len(), 5);
    assert_eq!(r.next_record_position, Some(6));
    assert!(r.diagnostics.is_empty());
    let first = &r.records[0];
    assert_eq!(first.title(), Some("Figaro (Paris. 1838)"));
    let ark = first.ark().unwrap();
    assert_eq!(ark.name(), "cb34447474s");
    assert!(ark.is_periodical());
    assert_eq!(
        first.representative_ark.as_ref().unwrap().name(),
        "bpt6k269136x"
    );
    assert_eq!(first.typedoc.as_deref(), Some("periodiques"));
    assert_eq!(first.ocr_quality, Some(87.32));
    assert_eq!(
        first.highres.as_deref(),
        Some("https://gallica.bnf.fr/ark:/12148/bpt6k269136x.highres")
    );
    assert_eq!(
        first.thumbnail.as_deref(),
        Some("https://gallica.bnf.fr/ark:/12148/bpt6k269136x.thumbnail")
    );
    let image = &r.records[4];
    assert_eq!(image.typedoc.as_deref(), Some("images"));
    assert_eq!(image.ark().unwrap().name(), "btv1b53128639d");
}

#[test]
fn content_search_decodes_escaped_html() {
    let r = document::parse_content_search(&fixture("contentsearch.xml")).unwrap();
    assert_eq!(r.count, 2);
    assert_eq!(r.items[0].page, Some(1));
    assert_eq!(
        r.items[0].highlights,
        vec!["PARIS", "PARIS", "PARIS", "Paris"]
    );
    assert!(
        r.items[0]
            .excerpt
            .starts_with("18 «t 20, rue dn Saint-Gotbard PARIS (14’) Chèque"),
        "{}",
        r.items[0].excerpt
    );
    assert!(!r.items[0].excerpt.contains('<'));
    assert!(!r.items[0].excerpt.contains("&#"));
    assert_eq!(r.items[1].page, Some(7));
    assert!(r.items[1].excerpt.contains("étranges voitures"));
}

#[test]
fn iiif_image_api_1_1() {
    let info: ImageInfo = serde_json::from_str(&fixture("info.json")).unwrap();
    assert_eq!((info.width, info.height), (4318, 6085));
    assert!(info.profile.as_str().unwrap().contains("image-api/1.1"));

    let m = Manifest::from_json(&fixture("manifest.json")).unwrap();
    assert_eq!(m.canvases.len(), 8);
    assert_eq!(
        m.label.as_deref(),
        Some("BnF, département Droit, économie, politique, JO-55958")
    );
    assert_eq!(
        m.license.as_deref(),
        Some("https://gallica.bnf.fr/html/und/conditions-dutilisation-des-contenus-de-gallica")
    );
    assert!(
        m.metadata
            .iter()
            .any(|(k, v)| k == "Title" && v == "Les Belles images")
    );
    let c = &m.canvases[0];
    assert_eq!(c.view, 1);
    assert_eq!((c.width, c.height), (Some(4318), Some(6085)));
    assert_eq!(
        c.service_id.as_deref(),
        Some("https://gallica.bnf.fr/iiif/ark:/12148/bpt6k961287k/f1")
    );
    assert_eq!(
        c.image_url.as_deref(),
        Some("https://gallica.bnf.fr/iiif/ark:/12148/bpt6k961287k/f1/full/full/0/native.jpg")
    );
    assert_eq!(
        c.thumbnail.as_deref(),
        Some("https://gallica.bnf.fr/ark:/12148/bpt6k961287k/f1.thumbnail")
    );
    assert_eq!(m.canvases[7].view, 8);
}

#[test]
fn toc_detection_is_not_fooled_by_xml_declaration() {
    let toc = document::parse_toc(fixture("pagination.xml"));
    assert_eq!(toc.format, TocFormat::Tei);
}
