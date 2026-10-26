# gallica

A bulk downloader for [Gallica](https://gallica.bnf.fr), the digital library of
the Bibliothèque nationale de France, written in Rust.

The project is built in layers:

1. **`crates/gallica`** — a client SDK for the Gallica APIs (this is what exists today);
2. a CLI on top of the SDK (next);
3. a cross-platform GUI (later).

## The applications

| Binary | Crate | What it is |
|---|---|---|
| `gallica` | `crates/gallica-tui` | headless commands (`info`, `plan`, `download`) and the terminal UI (`tui`, also the default) |
| `gallica-gui` | `crates/gallica-gui` | the desktop GUI (egui/eframe) |

Both are thin front ends over `crates/gallica-core`, the UI-agnostic
application core (state, actions, events, a pure reducer and an engine running
the SDK on tokio). The design is described in [docs/SPEC.md](docs/SPEC.md).

```sh
# What is it?
gallica info https://gallica.bnf.fr/ark:/12148/cb327110547/

# What would a download fetch, and what would it cost under the BnF quotas?
gallica plan https://gallica.bnf.fr/ark:/12148/cb327110547/ --years 1921-1925

# Download (highres previews by default; --variant full needs --yes)
gallica download https://gallica.bnf.fr/ark:/12148/cb327110547/ \
    --years 1921-1925 --out ~/Gallica --contact you@example.org

# Interactive
gallica tui
gallica-gui
```

Files land in `<out>/<ark> - <title>/<year>/<date> <ark>/f0001.jpg` with a
`metadata.json` per issue and a `done.json` marker; running the same target
again resumes without re-requesting finished issues. Settings (contact address,
output folder, image variant, quotas) persist in the platform configuration
directory as `config.toml`.

## The SDK

The `gallica` crate wraps the public BnF APIs documented on
[api.bnf.fr](https://api.bnf.fr/fr/recherche?f%5B0%5D=categories%3A1158):

| API | Module | What it gives you |
|-----|--------|-------------------|
| [Document](https://api.bnf.fr/fr/api-document-de-gallica) | `document` | `Issues` (years and issues of a periodical), `OAIRecord` (bibliographic record), `Pagination` (views of a document), `ContentSearch` (full text search inside a document), `Toc`, plain text (`texteBrut`), ALTO OCR, PDF and preview images |
| [Search](https://api.bnf.fr/fr/api-gallica-de-recherche) | `sru` | SRU 1.2 searches with CQL queries, paging, and a small CQL builder |
| [IIIF](https://api.bnf.fr/fr/api-iiif-de-recuperation-des-images-de-gallica) | `iiif` | Image API URLs (region, size, rotation, quality, format), `info.json`, Presentation manifests, image download |

On top of those, `collection` resolves any Gallica URL or ARK into either a
periodical or a document, enumerates the issues of a periodical and the views
of a document; `download` streams files to disk through a temporary file.

The client throttles requests (250 ms apart by default), bounds concurrent
downloads (2 by default) and retries transient failures (HTTP 429/5xx,
timeouts) with exponential backoff, honouring `Retry-After`. It also enforces
the [usage quotas published by the BnF](https://api.bnf.fr/fr/node/232) on
its own side: 5 full-resolution IIIF images, 50 `highres` previews, 4 PDFs
and 5 `texteBrut` exports per minute, and 832 KB/s of download bandwidth.
See `Quotas` to adjust them.

How page images are fetched is a single choice, `ImageVariant`: the
`highres` preview by default (about 1024×1536 px, 50 per minute), an IIIF image
capped to a width (up to 1000 px stays outside the strict quota), or the full
master resolution as an explicit opt-in (5 per minute, so a whole newspaper
title takes days). `Quotas::minimum_duration` estimates the time a given
number of requests needs under those limits.

```rust
use gallica::{Client, Resource, iiif::ImageRequest, download::DownloadOptions};

let client = Client::builder()
    .user_agent("my-downloader/0.1 (contact@example.org)")
    .build()?;

match Resource::parse("https://gallica.bnf.fr/ark:/12148/cb327110547/")? {
    Resource::Periodical { ark } => {
        for issue in client.periodical_issues(&ark, Some(1900..=1901)).await? {
            println!("{} {:?} {}", issue.ark, issue.date(), issue.label);
        }
    }
    Resource::Document { ark, .. } => {
        for view in client.document_views(&ark).await? {
            let path = format!("{}/f{}.jpg", ark.name(), view.view);
            client.download_iiif_image(&ark, view.view, &ImageRequest::full(), &path, &DownloadOptions::new()).await?;
        }
    }
}
```

Accepted inputs include bare names (`bpt6k961287k`), identifiers
(`ark:/12148/cb327110547/date`) and Gallica URLs with any qualifier
(`.../bpt6k961287k/f3.item.zoom?rk=...`, `.../bpt6k961287k.texteBrut`, IIIF
image URLs). Names starting with `cb` are periodical titles.

### Try it

```sh
cargo run --example inspect -- https://gallica.bnf.fr/ark:/12148/cb327110547/
cargo run --example inspect -- https://gallica.bnf.fr/ark:/12148/bpt6k961287k --download
```

## Building

With Nix (flakes), `nix develop` gives a shell with the toolchain pinned by
`rust-toolchain.toml` through [rust-overlay](https://github.com/oxalica/rust-overlay);
`nix build` builds and tests the command line and terminal UI (`gallica`),
`nix build .#gallica-gui` the desktop GUI. From Linux,
`nix build .#gallica-tui-windows` and `.#gallica-gui-windows` cross compile
self-contained Windows x86_64 executables (MinGW, tests not run). Without Nix,
any recent stable Rust
(edition 2024) works:

```sh
cargo build
cargo test
```

Tests run against a local mock server; nothing talks to gallica.bnf.fr.

## Being a good citizen

Gallica is a public service. Keep the default throttling, identify your
client with a `User-Agent` that includes a way to contact you, and read the
[conditions of use](https://gallica.bnf.fr/html/und/conditions-dutilisation-des-contenus-de-gallica).
