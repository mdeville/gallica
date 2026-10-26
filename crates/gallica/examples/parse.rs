//! Parses a Gallica API answer saved to a file and prints what the SDK makes
//! of it. Handy to validate the parsers on real answers saved from a browser.
//!
//! ```text
//! cargo run --example parse -- issues  issues.xml
//! cargo run --example parse -- issues  issues-1937.xml 1937
//! cargo run --example parse -- oai     oairecord.xml
//! cargo run --example parse -- pages   pagination.xml
//! cargo run --example parse -- sru     sru.xml
//! cargo run --example parse -- search  contentsearch.xml
//! cargo run --example parse -- toc     toc.xml
//! cargo run --example parse -- info    info.json
//! cargo run --example parse -- manifest manifest.json
//! ```

use gallica::document;
use gallica::iiif::{ImageInfo, Manifest};
use gallica::sru;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (Some(kind), Some(path)) = (args.first(), args.get(1)) else {
        eprintln!("usage: parse <issues|oai|pages|sru|search|toc|info|manifest> <file> [year]");
        std::process::exit(2);
    };
    let body = std::fs::read_to_string(path)?;

    match kind.as_str() {
        "issues" => match args.get(2) {
            Some(year) => {
                let list = document::parse_issue_list(&body, year.parse()?)?;
                println!(
                    "{} issues of {} in {}",
                    list.issues.len(),
                    list.parent_ark,
                    list.year
                );
                for i in &list.issues {
                    println!(
                        "  {:<16} {:<12} {}",
                        i.ark.name(),
                        i.date()
                            .map(|d| d.to_string())
                            .unwrap_or_else(|| "?".into()),
                        i.label
                    );
                }
            }
            None => {
                let years = document::parse_issue_years(&body)?;
                println!("{years:#?}");
            }
        },
        "oai" => println!("{:#?}", document::parse_oai_record(&body)?),
        "pages" => {
            let p = document::parse_pagination(&body)?;
            println!(
                "{} views (first displayed {:?}, toc {:?}, content {})",
                p.view_count(),
                p.first_displayed_page,
                p.toc_location,
                p.has_content
            );
            for page in p.pages.iter().take(10) {
                println!(
                    "  f{:<5} {:<6} {:?} {:?}",
                    page.order,
                    page.label.as_deref().unwrap_or(""),
                    page.pagination_type,
                    page.legend
                );
            }
            if p.pages.len() > 10 {
                println!("  ... {} more", p.pages.len() - 10);
            }
        }
        "sru" => {
            let r = sru::parse_search_results(&body)?;
            println!(
                "{} records total, {} in this page, next {:?}, diagnostics {:?}",
                r.number_of_records,
                r.records.len(),
                r.next_record_position,
                r.diagnostics
            );
            for rec in &r.records {
                println!(
                    "  {:<16} {:<12} {:<10} {}",
                    rec.ark()
                        .map(|a| a.name().to_string())
                        .unwrap_or_else(|| "?".into()),
                    rec.dc.date().unwrap_or(""),
                    rec.typedoc.as_deref().unwrap_or(""),
                    rec.title().unwrap_or("")
                );
            }
            if let Some(first) = r.records.first() {
                println!("extra record data of the first record: {:#?}", first.extra);
            }
        }
        "search" => println!("{:#?}", document::parse_content_search(&body)?),
        "toc" => {
            let toc = document::parse_toc(body);
            println!("{:?}, {} entries", toc.format, toc.entries.len());
            for e in toc.entries.iter().take(20) {
                println!(
                    "  f{:<5} {:<8} {}",
                    e.view.map(|v| v.to_string()).unwrap_or_default(),
                    e.page_label.as_deref().unwrap_or(""),
                    e.text
                );
            }
        }
        "info" => println!("{:#?}", serde_json::from_str::<ImageInfo>(&body)?),
        "manifest" => {
            let m = Manifest::from_json(&body)?;
            println!("{} ({:?}), {} canvases", m.id, m.label, m.canvases.len());
            println!("metadata: {:#?}", m.metadata);
            for c in m.canvases.iter().take(5) {
                println!(
                    "  f{:<5} {:?} {:?}x{:?} {:?}",
                    c.view, c.label, c.width, c.height, c.service_id
                );
            }
        }
        other => {
            eprintln!("unknown kind {other:?}");
            std::process::exit(2);
        }
    }
    Ok(())
}
