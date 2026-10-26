//! Dublin Core records as embedded in SRU and OAIRecord answers.

use crate::ark::Ark;
use crate::xml::Element;

/// A Dublin Core (`oai_dc`) record. Every field may repeat, so each one is a
/// list, in document order.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub struct DublinCore {
    /// `dc:title`
    pub titles: Vec<String>,
    /// `dc:creator`
    pub creators: Vec<String>,
    /// `dc:contributor`
    pub contributors: Vec<String>,
    /// `dc:publisher`
    pub publishers: Vec<String>,
    /// `dc:date`
    pub dates: Vec<String>,
    /// `dc:description`
    pub descriptions: Vec<String>,
    /// `dc:format`
    pub formats: Vec<String>,
    /// `dc:identifier`
    pub identifiers: Vec<String>,
    /// `dc:language`
    pub languages: Vec<String>,
    /// `dc:relation`
    pub relations: Vec<String>,
    /// `dc:rights`
    pub rights: Vec<String>,
    /// `dc:source`
    pub sources: Vec<String>,
    /// `dc:subject`
    pub subjects: Vec<String>,
    /// `dc:type`
    pub types: Vec<String>,
    /// `dc:coverage`
    pub coverage: Vec<String>,
    /// Any other element found in the record, as `(name, text)`.
    pub other: Vec<(String, String)>,
}

impl DublinCore {
    /// Reads the fields of an `oai_dc:dc` element.
    pub fn from_element(dc: &Element) -> DublinCore {
        let mut out = DublinCore::default();
        for e in dc.elements() {
            let text = e.text();
            if text.is_empty() {
                continue;
            }
            let bucket = match e.name.as_str() {
                "title" => &mut out.titles,
                "creator" => &mut out.creators,
                "contributor" => &mut out.contributors,
                "publisher" => &mut out.publishers,
                "date" => &mut out.dates,
                "description" => &mut out.descriptions,
                "format" => &mut out.formats,
                "identifier" => &mut out.identifiers,
                "language" => &mut out.languages,
                "relation" => &mut out.relations,
                "rights" => &mut out.rights,
                "source" => &mut out.sources,
                "subject" => &mut out.subjects,
                "type" => &mut out.types,
                "coverage" => &mut out.coverage,
                other => {
                    out.other.push((other.to_string(), text));
                    continue;
                }
            };
            bucket.push(text);
        }
        out
    }

    /// The first title, if any.
    pub fn title(&self) -> Option<&str> {
        self.titles.first().map(String::as_str)
    }

    /// The first date, if any.
    pub fn date(&self) -> Option<&str> {
        self.dates.first().map(String::as_str)
    }

    /// The Gallica ARK found among the identifiers.
    pub fn ark(&self) -> Option<Ark> {
        self.identifiers
            .iter()
            .filter(|id| id.contains("gallica.bnf.fr/ark:/"))
            .find_map(|id| Ark::parse(id).ok())
    }

    /// The number of views, parsed from the `Nombre total de vues : N` format.
    pub fn view_count(&self) -> Option<u32> {
        self.formats.iter().find_map(|f| {
            let rest = f.strip_prefix("Nombre total de vues")?;
            rest.trim_start_matches([' ', ':']).trim().parse().ok()
        })
    }

    /// The URL of the catalogue record (`Notice du catalogue : ...`), if any.
    pub fn catalogue_record(&self) -> Option<&str> {
        self.relations.iter().find_map(|r| {
            let (_, url) = r.split_once("Notice du catalogue")?;
            let url = url.trim_start_matches([' ', ':']).trim();
            (!url.is_empty()).then_some(url)
        })
    }

    /// The periodical this issue belongs to, from a relation such as
    /// `http://gallica.bnf.fr/ark:/12148/cb327110547/date`.
    pub fn periodical_ark(&self) -> Option<Ark> {
        self.relations
            .iter()
            .filter_map(|r| Ark::parse(r.trim()).ok())
            .find(|a| a.is_periodical())
    }

    /// `true` if the record declares itself as public domain.
    pub fn is_public_domain(&self) -> bool {
        self.rights.iter().any(|r| {
            r.eq_ignore_ascii_case("public domain") || r.eq_ignore_ascii_case("domaine public")
        })
    }
}
