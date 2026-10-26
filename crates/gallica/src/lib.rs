//! A client for the APIs of [Gallica](https://gallica.bnf.fr), the digital
//! library of the Bibliothèque nationale de France.
//!
//! The crate covers:
//!
//! - [`Ark`] parsing from bare names, `ark:/12148/...` identifiers and Gallica URLs;
//! - the **Document** API ([`document`]): `Issues`, `OAIRecord`, `Pagination`,
//!   `ContentSearch`, `Toc`, plain text, ALTO OCR, PDF and preview images;
//! - the **search** API ([`sru`]): SRU 1.2 with a small CQL builder;
//! - the **IIIF** Image and Presentation APIs ([`iiif`]);
//! - streaming downloads to disk with throttling, bounded concurrency and
//!   retries ([`download`]), within the usage quotas published by the BnF
//!   ([`quota`]);
//! - high level helpers to enumerate the issues of a periodical and the views
//!   of a document ([`collection`]).
//!
//! # Example
//!
//! ```no_run
//! use gallica::{Client, Resource, iiif::ImageRequest, download::DownloadOptions};
//!
//! # async fn run() -> gallica::Result<()> {
//! let client = Client::builder()
//!     .user_agent("my-downloader/0.1 (contact@example.org)")
//!     .build()?;
//!
//! match Resource::parse("https://gallica.bnf.fr/ark:/12148/cb327110547/")? {
//!     Resource::Periodical { ark } => {
//!         for issue in client.periodical_issues(&ark, Some(1900..=1901)).await? {
//!             println!("{} {:?} {}", issue.ark, issue.date(), issue.label);
//!         }
//!     }
//!     Resource::Document { ark, .. } => {
//!         for view in client.document_views(&ark).await? {
//!             let path = format!("{}/f{}.jpg", ark.name(), view.view);
//!             client
//!                 .download_iiif_image(&ark, view.view, &ImageRequest::full(), &path, &DownloadOptions::new())
//!                 .await?;
//!         }
//!     }
//! }
//! # Ok(()) }
//! ```

#![warn(missing_docs)]
#![forbid(unsafe_code)]

pub mod ark;
pub mod client;
pub mod collection;
pub mod dc;
pub mod document;
pub mod download;
pub mod error;
pub mod iiif;
pub mod quota;
pub mod sru;
pub mod xml;

pub use ark::{Ark, ArkKind, BNF_NAAN, GALLICA_BASE_URL};
pub use client::{Client, ClientBuilder, DEFAULT_USER_AGENT, RetryPolicy};
pub use collection::{Resource, View};
pub use dc::DublinCore;
pub use error::{ArkError, Error, Result};
pub use quota::{QuotaClass, Quotas};
