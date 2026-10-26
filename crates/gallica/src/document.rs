//! The Gallica *Document* API: `Issues`, `OAIRecord`, `Pagination`,
//! `ContentSearch`, `Toc`, plain text, ALTO OCR, PDF and preview images.
//!
//! See <https://api.bnf.fr/fr/api-document-de-gallica>.

use bytes::Bytes;
use url::Url;

use crate::ark::Ark;
use crate::client::Client;
use crate::dc::DublinCore;
use crate::download::{DownloadOptions, DownloadOutcome};
use crate::error::{Error, Result};
use crate::quota::QuotaClass;
use crate::xml::{Element, parse_bool};

// ---------------------------------------------------------------------------
// Issues
// ---------------------------------------------------------------------------

/// The years for which a periodical has at least one issue online.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct IssueYears {
    /// The periodical ARK as echoed by the service.
    pub parent_ark: String,
    /// Total number of issues for the title.
    pub total_issues: Option<u32>,
    /// `true` for "UC3" periodicals (a calendar can be built).
    pub uc3: Option<bool>,
    /// Years with at least one issue, ascending.
    pub years: Vec<u16>,
}

/// One issue of a periodical.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Issue {
    /// The ARK of the digitised issue.
    pub ark: Ark,
    /// Publication year (the one that was queried).
    pub year: u16,
    /// Day of the year (1-based), when known.
    pub day_of_year: Option<u16>,
    /// Free text label, usually `YYYY/MM/DD (A1,N2).` but not guaranteed.
    pub label: String,
}

impl Issue {
    /// The publication date computed from the year and day of year.
    pub fn date(&self) -> Option<Date> {
        Date::from_day_of_year(self.year, self.day_of_year?)
    }
}

/// A calendar date.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Date {
    /// Year.
    pub year: u16,
    /// Month, 1-12.
    pub month: u8,
    /// Day of month, 1-31.
    pub day: u8,
}

impl Date {
    /// Builds a date from a 1-based day of year.
    pub fn from_day_of_year(year: u16, day_of_year: u16) -> Option<Date> {
        let leap =
            (year.is_multiple_of(4) && !year.is_multiple_of(100)) || year.is_multiple_of(400);
        let lengths = [
            31,
            if leap { 29 } else { 28 },
            31,
            30,
            31,
            30,
            31,
            31,
            30,
            31,
            30,
            31,
        ];
        let mut remaining = day_of_year;
        if remaining == 0 {
            return None;
        }
        for (i, len) in lengths.iter().enumerate() {
            if remaining <= *len {
                return Some(Date {
                    year,
                    month: i as u8 + 1,
                    day: remaining as u8,
                });
            }
            remaining -= len;
        }
        None
    }
}

impl std::fmt::Display for Date {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:04}-{:02}-{:02}", self.year, self.month, self.day)
    }
}

/// The issues of a periodical for one year.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct IssueList {
    /// The periodical ARK as echoed by the service.
    pub parent_ark: String,
    /// The year that was queried.
    pub year: u16,
    /// Issues, sorted by publication day.
    pub issues: Vec<Issue>,
}

// ---------------------------------------------------------------------------
// OAIRecord
// ---------------------------------------------------------------------------

/// The bibliographic record of a document, from the `OAIRecord` service.
#[derive(Debug, Clone, Default, PartialEq)]
#[non_exhaustive]
pub struct OaiRecord {
    /// OAI identifier, e.g. `oai:bnf.fr:gallica/ark:/12148/bpt6k5738219s`.
    pub identifier: Option<String>,
    /// OAI datestamp (`YYYY-MM-DD`).
    pub datestamp: Option<String>,
    /// OAI sets, e.g. `gallica:typedoc:monographies`.
    pub set_specs: Vec<String>,
    /// The Dublin Core record.
    pub dc: DublinCore,
    /// Document type: `monographies`, `fascicules`, `images`, `sonore`, `video`...
    pub typedoc: Option<String>,
    /// Average OCR quality (0-100). Full text search is available above ~50.
    pub ocr_quality: Option<f32>,
    /// Digital provenance (`bnf.fr` for BnF scans).
    pub provenance: Option<String>,
    /// Physical source / shelfmark.
    pub source: Option<String>,
    /// Display title.
    pub title: Option<String>,
    /// Display date.
    pub date: Option<String>,
    /// Number of issues (`nbIssue` attribute of `<date>`).
    pub nb_issue: Option<u32>,
    /// `text` when the document is OCRed and indexed.
    pub mode_indexation: Option<String>,
    /// Dewey sub-class.
    pub sdewey: Option<String>,
    /// Date of first indexation (`DD/MM/YYYY`).
    pub first_indexation_date: Option<String>,
    /// Visibility rights (`all`...).
    pub visibility_rights: Option<String>,
    /// Audio tracks, for sound documents.
    pub sounds: Vec<MediaTrack>,
    /// Video file URL, for moving images.
    pub video: Option<String>,
}

impl OaiRecord {
    /// The document ARK, from the Dublin Core identifiers or the OAI header.
    pub fn ark(&self) -> Option<Ark> {
        self.dc.ark().or_else(|| {
            self.identifier
                .as_deref()
                .and_then(|id| id.split_once("gallica/"))
                .and_then(|(_, rest)| Ark::parse(rest).ok())
        })
    }

    /// `true` when the document has indexed full text.
    pub fn has_full_text(&self) -> bool {
        self.mode_indexation.as_deref() == Some("text")
            || self.ocr_quality.map(|q| q >= 50.0).unwrap_or(false)
    }
}

/// An audio track attached to a page.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct MediaTrack {
    /// Page (view) number the track belongs to.
    pub page: Option<u32>,
    /// Track title.
    pub title: Option<String>,
    /// File URL.
    pub file: String,
}

// ---------------------------------------------------------------------------
// Pagination
// ---------------------------------------------------------------------------

/// The structure and page list of a document, from the `Pagination` service.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct Pagination {
    /// The view Gallica opens by default (title page, cover). `<1` means first.
    pub first_displayed_page: Option<i32>,
    /// Whether the document has a table of contents (`Toc` service usable).
    pub has_toc: bool,
    /// View number of the table of contents, if any.
    pub toc_location: Option<u32>,
    /// Whether the document has indexed text.
    pub has_content: bool,
    /// Shelfmark of the digital document (`NUMM-...`).
    pub id_upn: Option<String>,
    /// Number of image views.
    pub nb_vue_images: Option<u32>,
    /// Number of audio views (audio documents only).
    pub nb_vue_audio: Option<u32>,
    /// The pages, in view order.
    pub pages: Vec<Page>,
}

impl Pagination {
    /// The number of views: `nbVueImages` when present, else the page count.
    pub fn view_count(&self) -> u32 {
        self.nb_vue_images.unwrap_or(self.pages.len() as u32)
    }
}

/// One page (view) of a document.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Page {
    /// Sequential view number; the value to use in `f<N>` qualifiers.
    pub order: u32,
    /// Printed page label (`NP` when unnumbered).
    pub label: Option<String>,
    /// Type of the label: `N` (none), `A` (arabic), `R` (roman).
    pub pagination_type: Option<String>,
    /// Caption, for maps, image sets and manuscripts.
    pub legend: Option<String>,
    /// Width of the master image in pixels, when the service provides it.
    pub width: Option<u32>,
    /// Height of the master image in pixels, when the service provides it.
    pub height: Option<u32>,
}

impl Page {
    /// `true` if the page carries a printed number.
    pub fn is_numbered(&self) -> bool {
        self.pagination_type
            .as_deref()
            .map(|t| t != "N")
            .unwrap_or(false)
            && self.label.as_deref().map(|l| l != "NP").unwrap_or(false)
    }
}

// ---------------------------------------------------------------------------
// ContentSearch
// ---------------------------------------------------------------------------

/// Results of a full text search inside one document.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct ContentSearchResults {
    /// The query as echoed by the service.
    pub query: String,
    /// Number of matching pages (or words, when searching one page).
    pub count: u32,
    /// Matches.
    pub items: Vec<ContentMatch>,
}

/// A match of a full text search.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct ContentMatch {
    /// Page (view) number, parsed from `PAG_<n>`.
    pub page: Option<u32>,
    /// The page identifier as given (`PAG_173`).
    pub page_id: String,
    /// Plain text excerpt, with the highlighted words kept.
    pub excerpt: String,
    /// The highlighted words.
    pub highlights: Vec<String>,
    /// Width of the master image, when searching one page.
    pub page_width: Option<u32>,
    /// Height of the master image, when searching one page.
    pub page_height: Option<u32>,
    /// Word boxes on the master image, when searching one page.
    pub words: Vec<WordBox>,
}

/// The bounding box of an OCR word, in master image pixels.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct WordBox {
    /// ALTO string identifier.
    pub id: String,
    /// Left edge.
    pub x: u32,
    /// Top edge.
    pub y: u32,
    /// Width.
    pub width: u32,
    /// Height.
    pub height: u32,
}

// ---------------------------------------------------------------------------
// Table of contents
// ---------------------------------------------------------------------------

/// Format of a table of contents answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TocFormat {
    /// Legacy HTML with `allerA(...)` links.
    Html,
    /// TEI XML (`tdmNum`).
    Tei,
}

/// A table of contents.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Toc {
    /// Which format the service returned.
    pub format: TocFormat,
    /// The raw body.
    pub raw: String,
    /// Best effort list of entries.
    pub entries: Vec<TocEntry>,
}

/// One entry of a table of contents.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct TocEntry {
    /// Entry text.
    pub text: String,
    /// Printed page label as shown in the table.
    pub page_label: Option<String>,
    /// View number to use in `f<N>` qualifiers.
    pub view: Option<u32>,
}

/// Preview image sizes served by Gallica outside IIIF.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PreviewSize {
    /// One side is 128 or 192 px.
    Thumbnail,
    /// One side is 256 or 384 px.
    LowRes,
    /// One side is 512 or 768 px.
    MedRes,
    /// One side is 1024 or 1536 px.
    HighRes,
}

impl PreviewSize {
    /// The quota class: only `highres` previews are rate limited by the BnF.
    pub fn quota_class(self) -> QuotaClass {
        match self {
            PreviewSize::HighRes => QuotaClass::HighResPreview,
            _ => QuotaClass::Default,
        }
    }

    /// The qualifier used in URLs.
    pub fn qualifier(self) -> &'static str {
        match self {
            PreviewSize::Thumbnail => "thumbnail",
            PreviewSize::LowRes => "lowres",
            PreviewSize::MedRes => "medres",
            PreviewSize::HighRes => "highres",
        }
    }
}

// ---------------------------------------------------------------------------
// Client methods
// ---------------------------------------------------------------------------

impl Client {
    /// Lists the years for which a periodical has issues online.
    pub async fn issue_years(&self, periodical: &Ark) -> Result<IssueYears> {
        let url = self.service_url("Issues", &[("ark", &periodical.issues_id())])?;
        let xml = self.get_text(url).await?;
        parse_issue_years(&xml)
    }

    /// Lists the issues of a periodical for one year.
    pub async fn issues(&self, periodical: &Ark, year: u16) -> Result<IssueList> {
        let url = self.service_url(
            "Issues",
            &[
                ("ark", &periodical.issues_id()),
                ("date", &year.to_string()),
            ],
        )?;
        let xml = self.get_text(url).await?;
        parse_issue_list(&xml, year)
    }

    /// Fetches the bibliographic record of a document.
    pub async fn oai_record(&self, ark: &Ark) -> Result<OaiRecord> {
        let url = self.service_url("OAIRecord", &[("ark", ark.name())])?;
        let xml = self.get_text(url).await?;
        parse_oai_record(&xml)
    }

    /// Fetches the page structure of a document.
    pub async fn pagination(&self, ark: &Ark) -> Result<Pagination> {
        let url = self.service_url("Pagination", &[("ark", ark.name())])?;
        let xml = self.get_text(url).await?;
        parse_pagination(&xml)
    }

    /// Searches the full text of a document. Without `page`, returns the
    /// pages that match with an excerpt (10 per call, use `start_result` to
    /// page through). With `page`, returns the word boxes on that view.
    pub async fn content_search(
        &self,
        ark: &Ark,
        query: &str,
        page: Option<u32>,
        start_result: Option<u32>,
    ) -> Result<ContentSearchResults> {
        let page_s;
        let start_s;
        let mut params: Vec<(&str, &str)> = vec![("ark", ark.name()), ("query", query)];
        if let Some(p) = page {
            page_s = p.to_string();
            params.push(("page", &page_s));
        }
        if let Some(s) = start_result {
            start_s = s.to_string();
            params.push(("startResult", &start_s));
        }
        let url = self.service_url("ContentSearch", &params)?;
        let xml = self.get_text(url).await?;
        parse_content_search(&xml)
    }

    /// Fetches the table of contents of a document.
    pub async fn toc(&self, ark: &Ark) -> Result<Toc> {
        let url = self.service_url("Toc", &[("ark", &ark.id())])?;
        let body = self.get_text(url).await?;
        Ok(parse_toc(body))
    }

    /// URL of the plain text (`texteBrut`) of a document, optionally limited
    /// to `count` views starting at view `from`.
    pub fn text_url(&self, ark: &Ark, range: Option<(u32, u32)>) -> Result<Url> {
        match range {
            Some((from, count)) => self.url(&format!("{}/f{from}n{count}.texteBrut", ark.id())),
            None => self.url(&format!("{}.texteBrut", ark.id())),
        }
    }

    /// Fetches the plain text of a document (HTML wrapped, with a
    /// bibliographic header), optionally limited to a range of views.
    pub async fn text(&self, ark: &Ark, range: Option<(u32, u32)>) -> Result<String> {
        let resp = self
            .get_classed(self.text_url(ark, range)?, QuotaClass::Text)
            .await?;
        Ok(resp.text().await?)
    }

    /// URL of the ALTO OCR of one view.
    pub fn alto_url(&self, ark: &Ark, view: u32) -> Result<Url> {
        let mut url = self.url("RequestDigitalElement")?;
        url.query_pairs_mut()
            .append_pair("O", ark.name())
            .append_pair("E", "ALTO")
            .append_pair("Deb", &view.to_string());
        Ok(url)
    }

    /// Fetches the ALTO OCR XML of one view.
    pub async fn alto(&self, ark: &Ark, view: u32) -> Result<String> {
        self.get_text(self.alto_url(ark, view)?).await
    }

    /// URL of the PDF of a document, optionally limited to `count` views
    /// starting at view `from`.
    pub fn pdf_url(&self, ark: &Ark, range: Option<(u32, u32)>) -> Result<Url> {
        match range {
            Some((from, count)) => self.url(&format!("{}/f{from}n{count}.pdf", ark.id())),
            None => self.url(&format!("{}.pdf", ark.id())),
        }
    }

    /// Fetches the PDF of a document into memory. Prefer
    /// [`Client::download`] for large documents.
    pub async fn pdf(&self, ark: &Ark, range: Option<(u32, u32)>) -> Result<Bytes> {
        let _permit = self.download_permit().await;
        let resp = self
            .get_classed(self.pdf_url(ark, range)?, QuotaClass::Pdf)
            .await?;
        self.read_body(resp).await
    }

    /// Downloads the PDF of a document to a file. See [`Client::download`]
    /// for the behaviour of `options`.
    pub async fn download_pdf(
        &self,
        ark: &Ark,
        range: Option<(u32, u32)>,
        path: impl AsRef<std::path::Path>,
        options: &DownloadOptions,
    ) -> Result<DownloadOutcome> {
        self.download_classed(self.pdf_url(ark, range)?, path, options, QuotaClass::Pdf)
            .await
    }

    /// URL of a pre-computed preview image of one view.
    pub fn preview_url(&self, ark: &Ark, view: u32, size: PreviewSize) -> Result<Url> {
        self.url(&format!("{}/f{view}.{}", ark.id(), size.qualifier()))
    }

    /// Fetches a pre-computed preview image of one view.
    pub async fn preview(&self, ark: &Ark, view: u32, size: PreviewSize) -> Result<Bytes> {
        let resp = self
            .get_classed(self.preview_url(ark, view, size)?, size.quota_class())
            .await?;
        self.read_body(resp).await
    }
}

// ---------------------------------------------------------------------------
// Parsers
// ---------------------------------------------------------------------------

/// Parses an `Issues` answer without a `date` parameter.
pub fn parse_issue_years(xml: &str) -> Result<IssueYears> {
    let root = Element::parse(xml, "Issues")?;
    if root.name != "issues" {
        return Err(Error::xml(
            "Issues",
            format!("unexpected root <{}>", root.name),
        ));
    }
    let mut years: Vec<u16> = root
        .children("year")
        .filter_map(|e| e.parse_text())
        .collect();
    years.sort_unstable();
    years.dedup();
    Ok(IssueYears {
        parent_ark: root
            .attr_any(&["parent_ark", "parentArk"])
            .unwrap_or_default()
            .to_string(),
        total_issues: root
            .attr_any(&["total_issues", "totalIssues"])
            .and_then(|s| s.parse().ok()),
        uc3: root.attr("uc3").and_then(parse_bool),
        years,
    })
}

/// Parses an `Issues` answer for one year.
pub fn parse_issue_list(xml: &str, year: u16) -> Result<IssueList> {
    let root = Element::parse(xml, "Issues")?;
    if root.name != "issues" {
        return Err(Error::xml(
            "Issues",
            format!("unexpected root <{}>", root.name),
        ));
    }
    let year = root
        .attr("date")
        .and_then(|d| d.parse().ok())
        .unwrap_or(year);
    let mut issues = Vec::new();
    for e in root.children("issue") {
        let Some(name) = e.attr("ark") else { continue };
        let ark = Ark::parse(name).map_err(|err| Error::xml("Issues", err))?;
        issues.push(Issue {
            ark,
            year,
            day_of_year: e.attr("dayOfYear").and_then(|s| s.parse().ok()),
            label: e.text(),
        });
    }
    Ok(IssueList {
        parent_ark: root
            .attr_any(&["parent_ark", "parentArk"])
            .unwrap_or_default()
            .to_string(),
        year,
        issues,
    })
}

/// Parses an `OAIRecord` answer.
pub fn parse_oai_record(xml: &str) -> Result<OaiRecord> {
    let root = Element::parse(xml, "OAIRecord")?;
    if root.name != "results" {
        return Err(Error::xml(
            "OAIRecord",
            format!("unexpected root <{}>", root.name),
        ));
    }
    if root
        .attr("countResults")
        .map(|c| c.trim() == "0")
        .unwrap_or(false)
        || root.child("notice").is_none()
    {
        return Err(Error::NotFound("OAIRecord: no notice in answer".into()));
    }
    let mut rec = OaiRecord::default();
    if let Some(header) = root.find("header") {
        rec.identifier = header.child_text("identifier");
        rec.datestamp = header.child_text("datestamp");
        rec.set_specs = header.children("setSpec").map(|e| e.text()).collect();
    }
    if let Some(dc) = root.find("dc") {
        rec.dc = DublinCore::from_element(dc);
    }
    rec.typedoc = root.child_text("typedoc");
    rec.ocr_quality = root.child("nqamoyen").and_then(|e| e.parse_text());
    rec.provenance = root.child_text("provenance");
    rec.source = root.child_text("source");
    rec.title = root.child_text("title");
    if let Some(d) = root.child("date") {
        rec.date = Some(d.text());
        rec.nb_issue = d.attr("nbIssue").and_then(|s| s.parse().ok());
    }
    rec.mode_indexation = root.child_text("mode_indexation");
    rec.sdewey = root.child_text("sdewey");
    rec.first_indexation_date = root.child_text("first_indexation_date");
    rec.visibility_rights = root.child_text("visibility_rights");
    if let Some(sounds) = root.child("sounds") {
        for page in sounds.children("page") {
            let num = page.attr("num").and_then(|s| s.parse().ok());
            for media in page.children("media") {
                if let Some(file) = media.child_text("file") {
                    rec.sounds.push(MediaTrack {
                        page: num,
                        title: media.child_text("title"),
                        file,
                    });
                }
            }
        }
    }
    rec.video = root.child("video").and_then(|v| v.child_text("file"));
    Ok(rec)
}

/// Parses a `Pagination` answer.
pub fn parse_pagination(xml: &str) -> Result<Pagination> {
    let root = Element::parse(xml, "Pagination")?;
    if root.name != "livre" {
        return Err(Error::xml(
            "Pagination",
            format!("unexpected root <{}>", root.name),
        ));
    }
    let mut out = Pagination::default();
    if let Some(s) = root.child("structure") {
        out.first_displayed_page = s.child("firstDisplayedPage").and_then(|e| e.parse_text());
        out.has_toc = s
            .child_text("hasToc")
            .as_deref()
            .and_then(parse_bool)
            .unwrap_or(false);
        out.toc_location = s
            .child("TocLocation")
            .and_then(|e| e.parse_text::<u32>())
            .filter(|loc| *loc > 0);
        out.has_content = s
            .child_text("hasContent")
            .as_deref()
            .and_then(parse_bool)
            .unwrap_or(false);
        out.id_upn = s.child_text("idUPN");
        out.nb_vue_images = s.child("nbVueImages").and_then(|e| e.parse_text());
        out.nb_vue_audio = s.child("nbVueAudio").and_then(|e| e.parse_text());
    }
    if let Some(pages) = root.child("pages") {
        for (i, p) in pages.children("page").enumerate() {
            let order = p
                .child("ordre")
                .and_then(|e| e.parse_text())
                .unwrap_or(i as u32 + 1);
            out.pages.push(Page {
                order,
                label: p.child_text("numero"),
                pagination_type: p.child_text("pagination_type"),
                legend: p.child_text("legend"),
                width: p.child("image_width").and_then(|e| e.parse_text()),
                height: p.child("image_height").and_then(|e| e.parse_text()),
            });
        }
    }
    Ok(out)
}

/// Parses a `ContentSearch` answer.
pub fn parse_content_search(xml: &str) -> Result<ContentSearchResults> {
    let root = Element::parse(xml, "ContentSearch")?;
    if root.name != "results" {
        return Err(Error::xml(
            "ContentSearch",
            format!("unexpected root <{}>", root.name),
        ));
    }
    let mut out = ContentSearchResults {
        query: root.child_text("query").unwrap_or_default(),
        count: root
            .attr("countResults")
            .and_then(|s| s.parse().ok())
            .unwrap_or(0),
        items: Vec::new(),
    };
    if let Some(items) = root.child("items") {
        for item in items.children("item") {
            let page_id = item.child_text("p_id").unwrap_or_default();
            let page = page_id.rsplit('_').next().and_then(|n| n.parse().ok());
            let (excerpt, highlights) = item
                .child("content")
                .map(excerpt_and_highlights)
                .unwrap_or_default();
            let mut words = Vec::new();
            if let Some(altoid) = item.child("altoid") {
                for w in altoid.children("altoidstring") {
                    let num = |n: &str| w.attr(n).and_then(|s| s.parse().ok());
                    if let (Some(x), Some(y), Some(width), Some(height)) =
                        (num("hpos"), num("vpos"), num("width"), num("height"))
                    {
                        words.push(WordBox {
                            id: w.text(),
                            x,
                            y,
                            width,
                            height,
                        });
                    }
                }
            }
            out.items.push(ContentMatch {
                page,
                page_id,
                excerpt,
                highlights,
                page_width: item.child("p_width").and_then(|e| e.parse_text()),
                page_height: item.child("p_height").and_then(|e| e.parse_text()),
                words,
            });
        }
    }
    Ok(out)
}

/// Parses a `Toc` answer in either of its formats.
pub fn parse_toc(raw: String) -> Toc {
    let head: String = raw
        .trim_start()
        .chars()
        .take(512)
        .collect::<String>()
        .to_ascii_lowercase();
    let is_html =
        head.starts_with("<!doctype html") || head.starts_with("<html") || head.contains("<html");
    if is_html {
        let entries = parse_toc_html(&raw);
        Toc {
            format: TocFormat::Html,
            raw,
            entries,
        }
    } else {
        let entries = parse_toc_tei(&raw);
        Toc {
            format: TocFormat::Tei,
            raw,
            entries,
        }
    }
}

fn parse_toc_html(html: &str) -> Vec<TocEntry> {
    // <div ...> ENTRY TEXT <a href="javascript:allerA('0083037', '140')">108</a>. = <a ...>111</a> </div>
    let link = regex::Regex::new(r#"(?s)allerA\('[^']*',\s*'(\d+)'\)"[^>]*>(.*?)</a>"#)
        .expect("valid regex");
    let div = regex::Regex::new(r"(?s)<div[^>]*>(.*?)</div>").expect("valid regex");
    let tag = regex::Regex::new(r"<[^>]+>").expect("valid regex");
    let mut entries = Vec::new();
    for d in div.captures_iter(html) {
        let inner = &d[1];
        let mut text_end = inner.len();
        let mut links = Vec::new();
        for l in link.captures_iter(inner) {
            let whole = l.get(0).unwrap();
            // The entry text is what precedes the first link (minus the "<a href=" prefix).
            if links.is_empty() {
                text_end = inner[..whole.start()].rfind("<a").unwrap_or(whole.start());
            }
            links.push((l[1].parse::<u32>().ok(), decode_html(&l[2])));
        }
        if links.is_empty() {
            continue;
        }
        let text = decode_html(&tag.replace_all(&inner[..text_end], " "));
        let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
        for (view, label) in links {
            entries.push(TocEntry {
                text: text.clone(),
                page_label: Some(label).filter(|l| !l.is_empty()),
                view,
            });
        }
    }
    entries
}

fn parse_toc_tei(xml: &str) -> Vec<TocEntry> {
    let Ok(root) = Element::parse(xml, "Toc") else {
        return Vec::new();
    };
    let view_re = regex::Regex::new(r"/(\d+)\.\w+\)?$").expect("valid regex");
    let mut entries = Vec::new();
    for row in root.find_all("row") {
        let segs = row.find_all("seg");
        let xrefs = row.find_all("xref");
        if segs.is_empty() && xrefs.is_empty() {
            continue;
        }
        let text = segs
            .iter()
            .map(|s| s.text())
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
        let xref = xrefs.first();
        let view = xref
            .and_then(|x| x.attr("from"))
            .and_then(|from| view_re.captures(from.trim_end_matches(')')))
            .and_then(|c| c[1].parse().ok());
        if text.is_empty() && view.is_none() {
            continue;
        }
        entries.push(TocEntry {
            text,
            page_label: xref.map(|x| x.text()).filter(|t| !t.is_empty()),
            view,
        });
    }
    entries
}

fn decode_html(s: &str) -> String {
    // Numeric entities first (`&#233;`, `&#xE9;`), then the named ones Gallica uses.
    let numeric = regex::Regex::new(r"&#(x[0-9a-fA-F]+|[0-9]+);").expect("valid regex");
    let s = numeric.replace_all(s, |c: &regex::Captures| {
        let code = &c[1];
        let value = if let Some(hex) = code.strip_prefix('x') {
            u32::from_str_radix(hex, 16).ok()
        } else {
            code.parse::<u32>().ok()
        };
        value
            .and_then(char::from_u32)
            .map(String::from)
            .unwrap_or_else(|| c[0].to_string())
    });
    let mut out = s
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&nbsp;", " ");
    for (ent, ch) in [
        ("&eacute;", 'é'),
        ("&egrave;", 'è'),
        ("&ecirc;", 'ê'),
        ("&euml;", 'ë'),
        ("&agrave;", 'à'),
        ("&acirc;", 'â'),
        ("&auml;", 'ä'),
        ("&icirc;", 'î'),
        ("&iuml;", 'ï'),
        ("&ocirc;", 'ô'),
        ("&ouml;", 'ö'),
        ("&ugrave;", 'ù'),
        ("&ucirc;", 'û'),
        ("&uuml;", 'ü'),
        ("&ccedil;", 'ç'),
        ("&oelig;", 'œ'),
        ("&aelig;", 'æ'),
        ("&Eacute;", 'É'),
        ("&Egrave;", 'È'),
        ("&Ecirc;", 'Ê'),
        ("&Agrave;", 'À'),
        ("&Acirc;", 'Â'),
        ("&Ccedil;", 'Ç'),
        ("&Ocirc;", 'Ô'),
        ("&OElig;", 'Œ'),
        ("&rsquo;", '’'),
        ("&laquo;", '«'),
        ("&raquo;", '»'),
    ] {
        if out.contains(ent) {
            out = out.replace(ent, &ch.to_string());
        }
    }
    out.trim().to_string()
}

/// Turns a `<content>` element into plain text plus the highlighted words.
///
/// The documentation shows the highlight `<span>`s as XML children, but the
/// live service escapes the HTML (`&lt;span ...&gt;`) and double-escapes
/// entities (`&amp;#233;`), so the text is run through an HTML pass either way.
fn excerpt_and_highlights(content: &Element) -> (String, Vec<String>) {
    let mut html = String::new();
    serialize_html(content, &mut html);
    let highlight =
        regex::Regex::new(r#"(?is)<span[^>]*class=['"]?highlight['"]?[^>]*>(.*?)</span>"#)
            .expect("valid regex");
    let highlights = highlight
        .captures_iter(&html)
        .map(|c| decode_html(&c[1]))
        .filter(|s| !s.is_empty())
        .collect();
    let tag = regex::Regex::new(r"<[^>]+>").expect("valid regex");
    let text = decode_html(&tag.replace_all(&html, ""));
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    (text, highlights)
}

/// Re-serialises real child elements as HTML so both shapes look alike.
fn serialize_html(el: &Element, out: &mut String) {
    for node in &el.children {
        match node {
            crate::xml::Node::Text(t) => {
                if !out.is_empty()
                    && !out.ends_with(char::is_whitespace)
                    && !t.starts_with(char::is_whitespace)
                {
                    out.push(' ');
                }
                out.push_str(t);
            }
            crate::xml::Node::Element(e) => {
                out.push('<');
                out.push_str(&e.name);
                for (k, v) in &e.attrs {
                    out.push_str(&format!(" {k}='{v}'"));
                }
                out.push('>');
                serialize_html(e, out);
                out.push_str(&format!("</{}>", e.name));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn date_from_day_of_year() {
        assert_eq!(
            Date::from_day_of_year(1937, 77).unwrap().to_string(),
            "1937-03-18"
        );
        assert_eq!(
            Date::from_day_of_year(1937, 364).unwrap().to_string(),
            "1937-12-30"
        );
        assert_eq!(
            Date::from_day_of_year(1936, 60).unwrap().to_string(),
            "1936-02-29"
        );
        assert_eq!(
            Date::from_day_of_year(1900, 60).unwrap().to_string(),
            "1900-03-01"
        );
        assert_eq!(Date::from_day_of_year(1937, 0), None);
        assert_eq!(Date::from_day_of_year(1937, 366), None);
    }

    #[test]
    fn toc_html_entries() {
        let html = r#"<html><body>
        <div class="Texte" style="font-size: 80%"> ABBEVILLE. Naud&eacute; y meurt, <a href="javascript:allerA('0083037', '140')">108</a>. = <a href="javascript:allerA('0083037', '143')">111</a>. </div>
        <div class="Titre2">No links</div></body></html>"#;
        let toc = parse_toc(html.to_string());
        assert_eq!(toc.format, TocFormat::Html);
        assert_eq!(toc.entries.len(), 2);
        assert_eq!(toc.entries[0].text, "ABBEVILLE. Naudé y meurt,");
        assert_eq!(toc.entries[0].view, Some(140));
        assert_eq!(toc.entries[0].page_label.as_deref(), Some("108"));
        assert_eq!(toc.entries[1].view, Some(143));
    }

    #[test]
    fn toc_tei_entries() {
        let xml = r#"<TEI.2><text id="tdm.9754046"><body><div0 type="TdM"><head>TABLE</head><div1><table>
          <row><cell/><cell>Pages</cell></row>
          <row><cell><seg n="int.000001">AVANT-PROPOS</seg></cell>
               <cell><xref n="lie.000001" type="image" from="FOREIGN(9754046/000011.jp2)">V</xref></cell></row>
        </table></div1></div0></body></text></TEI.2>"#;
        let toc = parse_toc(xml.to_string());
        assert_eq!(toc.format, TocFormat::Tei);
        assert_eq!(toc.entries.len(), 1);
        assert_eq!(toc.entries[0].text, "AVANT-PROPOS");
        assert_eq!(toc.entries[0].view, Some(11));
        assert_eq!(toc.entries[0].page_label.as_deref(), Some("V"));
    }
}
