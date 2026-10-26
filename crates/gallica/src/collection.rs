//! High level helpers that turn a user supplied URL into the list of
//! documents and views to download.

use std::ops::RangeInclusive;

use crate::ark::{Ark, ArkKind};
use crate::client::Client;
use crate::document::{Issue, Page};
use crate::error::{Error, Result};

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
#[derive(Debug, Clone, PartialEq, Eq)]
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
