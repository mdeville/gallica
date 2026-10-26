//! High level helpers that turn a user supplied URL into the list of
//! documents and views to download.

use std::ops::RangeInclusive;
use std::path::Path;

use bytes::Bytes;
use url::Url;

use crate::ark::{Ark, ArkKind};
use crate::client::Client;
use crate::document::{Issue, Page, PreviewSize};
use crate::download::{DownloadOptions, DownloadOutcome};
use crate::error::{Error, Result};
use crate::iiif;
use crate::quota::QuotaClass;

/// What a user supplied URL or identifier points at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resource {
    /// A periodical title; its issues must be enumerated year by year.
    Periodical {
        /// The periodical ARK.
        ark: Ark,
    },
    /// A single document, optionally a single view of it.
    Document {
        /// The document ARK.
        ark: Ark,
        /// The view selected in the input (`f<N>`), if any.
        view: Option<u32>,
    },
}

impl Resource {
    /// Parses a URL or identifier into a resource.
    pub fn parse(input: &str) -> Result<Resource> {
        let ark = Ark::parse(input)?;
        Ok(match ark.kind() {
            ArkKind::Periodical => Resource::Periodical {
                ark: ark.without_qualifier(),
            },
            ArkKind::Document => Resource::Document {
                view: ark.page(),
                ark: ark.without_qualifier(),
            },
        })
    }

    /// The ARK of the resource.
    pub fn ark(&self) -> &Ark {
        match self {
            Resource::Periodical { ark } | Resource::Document { ark, .. } => ark,
        }
    }
}

/// A view of a document, ready to be fetched.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub struct View {
    /// The document ARK.
    pub ark: Ark,
    /// 1-based view number.
    pub view: u32,
    /// Printed page label, if known.
    pub label: Option<String>,
    /// Caption, if any.
    pub legend: Option<String>,
    /// Master image width, when known from `Pagination`.
    pub width: Option<u32>,
    /// Master image height, when known from `Pagination`.
    pub height: Option<u32>,
}

impl Client {
    /// Lists every issue of a periodical, optionally restricted to a range of
    /// years. One `Issues` request is made per year.
    pub async fn periodical_issues(
        &self,
        periodical: &Ark,
        years: Option<RangeInclusive<u16>>,
    ) -> Result<Vec<Issue>> {
        let available = self.issue_years(periodical).await?;
        let mut out = Vec::new();
        for year in available.years {
            if let Some(r) = &years
                && !r.contains(&year)
            {
                continue;
            }
            let list = self.issues(periodical, year).await?;
            out.extend(list.issues);
        }
        Ok(out)
    }

    /// Lists the views of a document, using the `Pagination` service and
    /// falling back to the IIIF manifest when pagination is unavailable.
    pub async fn document_views(&self, ark: &Ark) -> Result<Vec<View>> {
        match self.pagination(ark).await {
            Ok(p) if !p.pages.is_empty() => Ok(p
                .pages
                .into_iter()
                .map(|page: Page| View {
                    ark: ark.without_qualifier(),
                    view: page.order,
                    label: page.label,
                    legend: page.legend,
                    width: page.width,
                    height: page.height,
                })
                .collect()),
            Ok(p) => {
                let n = p.view_count();
                if n > 0 {
                    return Ok((1..=n)
                        .map(|i| View {
                            ark: ark.without_qualifier(),
                            view: i,
                            label: None,
                            legend: None,
                            width: None,
                            height: None,
                        })
                        .collect());
                }
                self.views_from_manifest(ark).await
            }
            Err(e) => {
                tracing::debug!(%ark, error = %e, "pagination failed, trying the manifest");
                self.views_from_manifest(ark).await
            }
        }
    }

    async fn views_from_manifest(&self, ark: &Ark) -> Result<Vec<View>> {
        let m = self.iiif_manifest(ark).await?;
        if m.canvases.is_empty() {
            return Err(Error::NotFound(format!("{ark}: no views found")));
        }
        Ok(m.canvases
            .into_iter()
            .map(|c| View {
                ark: ark.without_qualifier(),
                view: c.view,
                label: c.label,
                legend: None,
                width: c.width,
                height: c.height,
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_inputs() {
        match Resource::parse("https://gallica.bnf.fr/ark:/12148/cb327110547/").unwrap() {
            Resource::Periodical { ark } => assert_eq!(ark.name(), "cb327110547"),
            other => panic!("unexpected {other:?}"),
        }
        match Resource::parse("https://gallica.bnf.fr/ark:/12148/bpt6k961287k/f4.item").unwrap() {
            Resource::Document { ark, view } => {
                assert_eq!(ark.name(), "bpt6k961287k");
                assert_eq!(ark.qualifier(), None);
                assert_eq!(view, Some(4));
            }
            other => panic!("unexpected {other:?}"),
        }
    }
}

/// How the image of a view is fetched.
///
/// Each variant maps to a URL, a quota class and a file extension, so that a
/// caller only has to choose once and let [`Client::download_view`] do the
/// rest. The BnF quotas make the choice matter:
///
/// | Variant | Typical size | BnF quota |
/// |---|---|---|
/// | [`ImageVariant::Full`] | master size, ~4300×6000 px for a newspaper page | 5 per minute |
/// | [`ImageVariant::Width`] above 1000 px | as requested | 5 per minute |
/// | [`ImageVariant::Width`] up to 1000 px | as requested | general throttle only |
/// | [`ImageVariant::Preview`] `HighRes` | fits in 1024×1536 px | 50 per minute |
/// | [`ImageVariant::Preview`] other sizes | 512×768 px and below | general throttle only |
///
/// The default is the `HighRes` preview: pre-computed on Gallica's side,
/// readable, and fast enough to fetch whole collections. Full resolution is an
/// explicit opt-in.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub enum ImageVariant {
    /// The full image at master resolution through IIIF.
    Full {
        /// Output format.
        format: iiif::Format,
    },
    /// The full image scaled to the given width through IIIF.
    Width {
        /// Width in pixels; the height follows the aspect ratio.
        width: u32,
        /// Output format.
        format: iiif::Format,
    },
    /// A pre-computed preview (`thumbnail`, `lowres`, `medres`, `highres`),
    /// always JPEG.
    Preview(PreviewSize),
    /// Any other IIIF request.
    Iiif(iiif::ImageRequest),
}

impl Default for ImageVariant {
    fn default() -> Self {
        ImageVariant::Preview(PreviewSize::HighRes)
    }
}

impl ImageVariant {
    /// Full resolution JPEG.
    pub fn full() -> Self {
        ImageVariant::Full {
            format: iiif::Format::Jpg,
        }
    }

    /// JPEG scaled to `width` pixels.
    pub fn width(width: u32) -> Self {
        ImageVariant::Width {
            width,
            format: iiif::Format::Jpg,
        }
    }

    /// The IIIF request this variant stands for, if it goes through IIIF.
    pub fn iiif_request(&self) -> Option<iiif::ImageRequest> {
        match *self {
            ImageVariant::Full { format } => Some(iiif::ImageRequest::full().format(format)),
            ImageVariant::Width { width, format } => Some(
                iiif::ImageRequest::full()
                    .size(iiif::Size::Width(width))
                    .format(format),
            ),
            ImageVariant::Iiif(req) => Some(req),
            ImageVariant::Preview(_) => None,
        }
    }

    /// The quota class requests of this variant are counted against.
    pub fn quota_class(&self) -> QuotaClass {
        match self {
            ImageVariant::Preview(size) => size.quota_class(),
            other => other
                .iiif_request()
                .map(|r| r.quota_class())
                .unwrap_or(QuotaClass::Default),
        }
    }

    /// The file extension of the fetched image.
    pub fn extension(&self) -> &'static str {
        match self.iiif_request() {
            Some(req) => req.format.extension(),
            None => "jpg",
        }
    }

    /// A file name for a view, zero padded so that files sort in view order:
    /// `f0001.jpg`.
    pub fn file_name(&self, view: u32) -> String {
        format!("f{view:04}.{}", self.extension())
    }

    /// `true` when the variant is counted against the strictest quota.
    pub fn is_full_resolution(&self) -> bool {
        self.quota_class() == QuotaClass::FullResImage
    }
}

impl Client {
    /// The URL of a view's image for the given variant.
    pub fn view_image_url(&self, ark: &Ark, view: u32, variant: &ImageVariant) -> Result<Url> {
        match variant {
            ImageVariant::Preview(size) => self.preview_url(ark, view, *size),
            other => self.iiif_image_url(
                ark,
                view,
                &other.iiif_request().expect("non-preview variants are IIIF"),
            ),
        }
    }

    /// Fetches the image of a view into memory.
    pub async fn view_image(&self, ark: &Ark, view: u32, variant: &ImageVariant) -> Result<Bytes> {
        match variant {
            ImageVariant::Preview(size) => self.preview(ark, view, *size).await,
            other => {
                self.iiif_image(
                    ark,
                    view,
                    &other.iiif_request().expect("non-preview variants are IIIF"),
                )
                .await
            }
        }
    }

    /// Downloads the image of a view to `path`. See [`Client::download`] for
    /// the behaviour of `options`.
    pub async fn download_view(
        &self,
        ark: &Ark,
        view: u32,
        variant: &ImageVariant,
        path: impl AsRef<Path>,
        options: &DownloadOptions,
    ) -> Result<DownloadOutcome> {
        let url = self.view_image_url(ark, view, variant)?;
        self.download_classed(url, path, options, variant.quota_class())
            .await
    }
}

#[cfg(test)]
mod variant_tests {
    use super::*;

    #[test]
    fn variants_map_to_urls_quotas_and_names() {
        let c = Client::new();
        let ark = Ark::new("bpt6k961287k").unwrap();

        let v = ImageVariant::default();
        assert_eq!(v, ImageVariant::Preview(PreviewSize::HighRes));
        assert_eq!(
            c.view_image_url(&ark, 3, &v).unwrap().as_str(),
            "https://gallica.bnf.fr/ark:/12148/bpt6k961287k/f3.highres"
        );
        assert_eq!(v.quota_class(), QuotaClass::HighResPreview);
        assert_eq!(v.file_name(3), "f0003.jpg");
        assert!(!v.is_full_resolution());

        let v = ImageVariant::full();
        assert_eq!(
            c.view_image_url(&ark, 12, &v).unwrap().as_str(),
            "https://gallica.bnf.fr/iiif/ark:/12148/bpt6k961287k/f12/full/full/0/native.jpg"
        );
        assert!(v.is_full_resolution());

        let v = ImageVariant::width(1000);
        assert_eq!(
            c.view_image_url(&ark, 1, &v).unwrap().as_str(),
            "https://gallica.bnf.fr/iiif/ark:/12148/bpt6k961287k/f1/full/1000,/0/native.jpg"
        );
        assert_eq!(v.quota_class(), QuotaClass::Default);
        assert!(ImageVariant::width(1001).is_full_resolution());

        let v = ImageVariant::Width {
            width: 800,
            format: iiif::Format::Png,
        };
        assert_eq!(v.extension(), "png");
        assert_eq!(v.file_name(42), "f0042.png");

        assert_eq!(
            ImageVariant::Preview(PreviewSize::MedRes).quota_class(),
            QuotaClass::Default
        );
    }
}
