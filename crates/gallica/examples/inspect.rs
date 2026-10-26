//! Prints what the SDK can find out about a Gallica URL or ARK, and
//! optionally downloads the first view as a full resolution JPEG.
//!
//! ```text
//! cargo run --example inspect -- https://gallica.bnf.fr/ark:/12148/cb327110547/
//! cargo run --example inspect -- https://gallica.bnf.fr/ark:/12148/bpt6k961287k --download
//! ```

use gallica::download::DownloadOptions;
use gallica::iiif::ImageRequest;
use gallica::{Client, Resource};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(input) = args.first() else {
        eprintln!("usage: inspect <gallica url or ark> [--download]");
        std::process::exit(2);
    };
    let download = args.iter().any(|a| a == "--download");

    let client = Client::builder()
        .user_agent("gallica-rs example (https://github.com/mdeville/gallica)")
        .build()?;

    match Resource::parse(input)? {
        Resource::Periodical { ark } => {
            println!("periodical {ark}");
            let years = client.issue_years(&ark).await?;
            println!(
                "  {} issues, years {:?}..{:?}",
                years
                    .total_issues
                    .map(|n| n.to_string())
                    .unwrap_or_else(|| "?".into()),
                years.years.first(),
                years.years.last()
            );
            if let Some(&year) = years.years.first() {
                let list = client.issues(&ark, year).await?;
                println!("  {year}: {} issues", list.issues.len());
                for issue in list.issues.iter().take(5) {
                    println!(
                        "    {} {} {}",
                        issue.ark.name(),
                        issue
                            .date()
                            .map(|d| d.to_string())
                            .unwrap_or_else(|| "?".into()),
                        issue.label
                    );
                }
            }
        }
        Resource::Document { ark, view } => {
            println!("document {ark}");
            let record = client.oai_record(&ark).await?;
            println!("  title: {}", record.dc.title().unwrap_or("?"));
            println!("  date: {}", record.dc.date().unwrap_or("?"));
            println!("  type: {}", record.typedoc.as_deref().unwrap_or("?"));
            println!("  rights: {}", record.dc.rights.join(" / "));
            let views = client.document_views(&ark).await?;
            println!("  {} views", views.len());
            let first = view.unwrap_or(1);
            let info = client.iiif_info(&ark, first).await?;
            println!("  view {first}: {}x{} px", info.width, info.height);
            if download {
                let path = format!("{}-f{first}.jpg", ark.name());
                let out = client
                    .download_iiif_image(
                        &ark,
                        first,
                        &ImageRequest::full(),
                        &path,
                        &DownloadOptions::new(),
                    )
                    .await?;
                println!("  saved {} ({} bytes)", out.path.display(), out.bytes);
            }
        }
    }
    Ok(())
}
