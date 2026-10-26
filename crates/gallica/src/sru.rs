//! The Gallica search API (SRU 1.2 with CQL queries).
//!
//! See <https://api.bnf.fr/fr/api-gallica-de-recherche>.

use std::fmt;

use url::Url;

use crate::ark::Ark;
use crate::client::Client;
use crate::dc::DublinCore;
use crate::error::{Error, Result};
use crate::xml::Element;

/// Maximum number of records the server returns per call.
pub const MAX_RECORDS_PER_PAGE: u32 = 50;

/// A search request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchQuery {
    /// The CQL query.
    pub cql: String,
    /// 1-based position of the first record to return.
    pub start_record: u32,
    /// Records per page (1..=50).
    pub maximum_records: u32,
    /// Collapse multi-volume documents and periodicals (server default: true).
    pub collapsing: Option<bool>,
}

impl SearchQuery {
    /// A query with default paging (first page, 50 records).
    pub fn new(cql: impl Into<String>) -> Self {
        SearchQuery {
            cql: cql.into(),
            start_record: 1,
            maximum_records: MAX_RECORDS_PER_PAGE,
            collapsing: None,
        }
    }

    /// Sets the first record position (1-based).
    pub fn start_record(mut self, start: u32) -> Self {
        self.start_record = start.max(1);
        self
    }

    /// Sets the page size (clamped to 1..=50).
    pub fn maximum_records(mut self, n: u32) -> Self {
        self.maximum_records = n.clamp(1, MAX_RECORDS_PER_PAGE);
        self
    }

    /// Sets collapsing. `false` lists every volume / issue separately.
    pub fn collapsing(mut self, collapsing: bool) -> Self {
        self.collapsing = Some(collapsing);
        self
    }
}

impl<S: Into<String>> From<S> for SearchQuery {
    fn from(s: S) -> Self {
        SearchQuery::new(s)
    }
}

/// One page of search results.
#[derive(Debug, Clone, Default, PartialEq)]
#[non_exhaustive]
pub struct SearchResults {
    /// The CQL query as echoed by the server.
    pub query: String,
    /// Total number of matching records.
    pub number_of_records: u32,
    /// Position of the next record, when the server provides it.
    pub next_record_position: Option<u32>,
    /// The records of this page.
    pub records: Vec<SearchRecord>,
    /// Diagnostics reported by the server, if any.
    pub diagnostics: Vec<String>,
}

/// One search result.
#[derive(Debug, Clone, Default, PartialEq)]
#[non_exhaustive]
pub struct SearchRecord {
    /// 1-based position of the record in the whole result set.
    pub position: Option<u32>,
    /// The Dublin Core record.
    pub dc: DublinCore,
    /// Document type from the extra record data (`monographie`, `fascicule`...).
    pub typedoc: Option<String>,
    /// OCR quality from the extra record data.
    pub ocr_quality: Option<f32>,
    /// Thumbnail URL from the extra record data.
    pub thumbnail: Option<String>,
    /// Digital provenance from the extra record data.
    pub provenance: Option<String>,
    /// Preview image URLs (`lowres`, `medres`, `highres`) from the extra record data.
    pub lowres: Option<String>,
    /// See [`SearchRecord::lowres`].
    pub medres: Option<String>,
    /// See [`SearchRecord::lowres`].
    pub highres: Option<String>,
    /// For a collapsed periodical, the document (issue) whose previews are
    /// shown (`uri` in the extra record data); for a document, the document
    /// itself.
    pub representative_ark: Option<Ark>,
    /// Every element of `srw:extraRecordData`, as `(name, text)`.
    pub extra: Vec<(String, String)>,
}

impl SearchRecord {
    /// The ARK of the result.
    pub fn ark(&self) -> Option<Ark> {
        self.dc.ark().or_else(|| {
            self.extra
                .iter()
                .filter(|(k, _)| k == "link" || k == "uri" || k == "ark")
                .find_map(|(_, v)| Ark::parse(v).ok())
        })
    }

    /// The first title.
    pub fn title(&self) -> Option<&str> {
        self.dc.title()
    }
}

impl Client {
    /// Builds the URL of a search request.
    pub fn search_url(&self, query: &SearchQuery) -> Result<Url> {
        let mut url = self.url("SRU")?;
        {
            let mut q = url.query_pairs_mut();
            q.append_pair("operation", "searchRetrieve");
            q.append_pair("version", "1.2");
            q.append_pair("query", &query.cql);
            q.append_pair("startRecord", &query.start_record.to_string());
            q.append_pair("maximumRecords", &query.maximum_records.to_string());
            if let Some(c) = query.collapsing {
                q.append_pair("collapsing", if c { "true" } else { "false" });
            }
        }
        Ok(url)
    }

    /// Runs one search request and returns one page of results.
    pub async fn search(&self, query: impl Into<SearchQuery>) -> Result<SearchResults> {
        let query = query.into();
        let xml = self.get_text(self.search_url(&query)?).await?;
        parse_search_results(&xml)
    }

    /// Runs a search and follows the pages until `limit` records have been
    /// collected (or every record, when `limit` is `None`).
    pub async fn search_all(
        &self,
        query: impl Into<SearchQuery>,
        limit: Option<usize>,
    ) -> Result<Vec<SearchRecord>> {
        let mut query = query.into();
        let mut out = Vec::new();
        loop {
            let page = self.search(query.clone()).await?;
            let got = page.records.len();
            out.extend(page.records);
            if let Some(l) = limit
                && out.len() >= l
            {
                out.truncate(l);
                break;
            }
            let next = page
                .next_record_position
                .unwrap_or(query.start_record + got as u32);
            if got == 0 || next > page.number_of_records {
                break;
            }
            query.start_record = next;
        }
        Ok(out)
    }
}

/// Parses an SRU `searchRetrieveResponse`.
pub fn parse_search_results(xml: &str) -> Result<SearchResults> {
    let root = Element::parse(xml, "SRU")?;
    if root.name != "searchRetrieveResponse" {
        return Err(Error::xml(
            "SRU",
            format!("unexpected root <{}>", root.name),
        ));
    }
    let mut out = SearchResults {
        query: root
            .child("echoedSearchRetrieveRequest")
            .and_then(|e| e.child_text("query"))
            .unwrap_or_default(),
        number_of_records: root
            .child("numberOfRecords")
            .and_then(|e| e.parse_text())
            .unwrap_or(0),
        next_record_position: root
            .child("nextRecordPosition")
            .and_then(|e| e.parse_text()),
        records: Vec::new(),
        diagnostics: root
            .find_all("diagnostic")
            .into_iter()
            .map(|d| {
                let msg = d.child_text("message").unwrap_or_default();
                let details = d.child_text("details").unwrap_or_default();
                format!("{msg} {details}").trim().to_string()
            })
            .collect(),
    };
    if let Some(records) = root.child("records") {
        for r in records.children("record") {
            let mut rec = SearchRecord {
                position: r.child("recordPosition").and_then(|e| e.parse_text()),
                ..Default::default()
            };
            if let Some(dc) = r.child("recordData").and_then(|d| d.find("dc")) {
                rec.dc = DublinCore::from_element(dc);
            }
            if let Some(extra) = r.child("extraRecordData") {
                for e in extra.elements() {
                    rec.extra.push((e.name.clone(), e.text()));
                }
                rec.typedoc = extra.child_text("typedoc");
                rec.ocr_quality = extra.child("nqamoyen").and_then(|e| e.parse_text());
                rec.thumbnail = extra.child_text("thumbnail");
                rec.provenance = extra.child_text("provenance");
                rec.lowres = extra.child_text("lowres");
                rec.medres = extra.child_text("medres");
                rec.highres = extra.child_text("highres");
                rec.representative_ark = extra.child_text("uri").and_then(|u| Ark::parse(&u).ok());
            }
            out.records.push(rec);
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// CQL helpers
// ---------------------------------------------------------------------------

/// CQL relations supported by Gallica.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Relation {
    /// All the words.
    All,
    /// Any of the words.
    Any,
    /// Exact phrase (disables query expansion for the whole query).
    Adj,
    /// Equal (dates).
    Eq,
    /// Less than (dates).
    Lt,
    /// Less than or equal (dates).
    Le,
    /// Greater than (dates).
    Gt,
    /// Greater than or equal (dates).
    Ge,
}

impl fmt::Display for Relation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Relation::All => "all",
            Relation::Any => "any",
            Relation::Adj => "adj",
            Relation::Eq => "=",
            Relation::Lt => "<",
            Relation::Le => "<=",
            Relation::Gt => ">",
            Relation::Ge => ">=",
        })
    }
}

/// A small CQL expression builder.
///
/// ```
/// use gallica::sru::{Cql, Relation};
/// let q = Cql::term("dc.title", Relation::All, "figaro")
///     .and(Cql::term("dc.type", Relation::Any, "fascicule"))
///     .sort_by("dc.date/sort.ascending");
/// assert_eq!(q.to_string(), r#"(dc.title all "figaro") and (dc.type any "fascicule") sortby dc.date/sort.ascending"#);
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cql {
    expr: String,
    sort: Option<String>,
}

impl Cql {
    /// A raw CQL expression, used verbatim.
    pub fn raw(expr: impl Into<String>) -> Self {
        Cql {
            expr: expr.into(),
            sort: None,
        }
    }

    /// `index relation "value"`.
    pub fn term(index: &str, relation: Relation, value: &str) -> Self {
        let value = value.replace('"', "\\\"");
        Cql {
            expr: format!("({index} {relation} \"{value}\")"),
            sort: None,
        }
    }

    /// Search everything (`gallica all "..."`).
    pub fn anywhere(value: &str) -> Self {
        Cql::term("gallica", Relation::All, value)
    }

    /// Combines with `and`.
    pub fn and(self, other: Cql) -> Self {
        Cql {
            expr: format!("{} and {}", self.expr, other.expr),
            sort: self.sort.or(other.sort),
        }
    }

    /// Combines with `or`.
    pub fn or(self, other: Cql) -> Self {
        Cql {
            expr: format!("{} or {}", self.expr, other.expr),
            sort: self.sort.or(other.sort),
        }
    }

    /// Combines with `not`.
    pub fn not(self, other: Cql) -> Self {
        Cql {
            expr: format!("{} not {}", self.expr, other.expr),
            sort: self.sort.or(other.sort),
        }
    }

    /// Adds a sort clause, e.g. `dc.date/sort.ascending`.
    pub fn sort_by(mut self, sort: impl Into<String>) -> Self {
        self.sort = Some(sort.into());
        self
    }
}

impl fmt::Display for Cql {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.expr)?;
        if let Some(s) = &self.sort {
            write!(f, " sortby {s}")?;
        }
        Ok(())
    }
}

impl From<Cql> for SearchQuery {
    fn from(c: Cql) -> Self {
        SearchQuery::new(c.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cql_builder() {
        let q = Cql::anywhere("toto").or(Cql::term("dc.creator", Relation::Adj, "Hugo, Victor"));
        assert_eq!(
            q.to_string(),
            r#"(gallica all "toto") or (dc.creator adj "Hugo, Victor")"#
        );
        assert_eq!(Cql::raw("dc.date >= 1900").to_string(), "dc.date >= 1900");
    }

    #[test]
    fn query_clamps() {
        let q = SearchQuery::new("x").maximum_records(500).start_record(0);
        assert_eq!(q.maximum_records, 50);
        assert_eq!(q.start_record, 1);
    }
}
