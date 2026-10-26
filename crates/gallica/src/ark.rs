//! Archival Resource Key (ARK) identifiers as used by Gallica.
//!
//! Every Gallica document is identified by an ARK of the form
//! `ark:/12148/<name>`, optionally followed by a qualifier such as `/f3.item`,
//! `/date` or `.texteBrut`. [`Ark::parse`] accepts bare names, ARK identifiers
//! and full Gallica URLs.

use std::fmt;
use std::str::FromStr;

use url::Url;

use crate::error::ArkError;

/// The BnF name assigning authority number.
pub const BNF_NAAN: &str = "12148";

/// Default Gallica base URL.
pub const GALLICA_BASE_URL: &str = "https://gallica.bnf.fr/";

/// What kind of resource an ARK designates, inferred from its name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ArkKind {
    /// A periodical title (catalogue record, name starts with `cb`). Issues
    /// are listed through the `Issues` service.
    Periodical,
    /// A digitised document: a book, an issue of a periodical, an image set...
    Document,
}

/// A parsed ARK identifier.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct Ark {
    naan: String,
    name: String,
    qualifier: Option<String>,
}

impl Ark {
    /// Builds an ARK for a bare Gallica name, e.g. `bpt6k961287k`.
    pub fn new(name: impl Into<String>) -> Result<Self, ArkError> {
        let name = name.into();
        validate_name(&name)?;
        Ok(Ark {
            naan: BNF_NAAN.to_string(),
            name,
            qualifier: None,
        })
    }

    /// Parses an ARK from a bare name, an `ark:/12148/...` identifier or a
    /// Gallica URL.
    ///
    /// Accepted forms include:
    /// - `bpt6k961287k`
    /// - `12148/bpt6k961287k`
    /// - `ark:/12148/bpt6k961287k`
    /// - `ark:/12148/cb327110547/date`
    /// - `https://gallica.bnf.fr/ark:/12148/bpt6k961287k/f3.item.zoom`
    /// - `https://gallica.bnf.fr/ark:/12148/bpt6k961287k.texteBrut?rk=...`
    /// - `https://gallica.bnf.fr/iiif/ark:/12148/bpt6k961287k/f1/full/full/0/native.jpg`
    pub fn parse(input: &str) -> Result<Self, ArkError> {
        let input = input.trim();
        if input.is_empty() {
            return Err(ArkError::Empty);
        }

        // Strip a scheme + host if present.
        let mut rest = input;
        if let Some(scheme_end) = rest.find("://") {
            let scheme = &rest[..scheme_end];
            if scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https") {
                let url =
                    Url::parse(rest).map_err(|_| ArkError::Unrecognized(input.to_string()))?;
                let host = url.host_str().unwrap_or("").to_ascii_lowercase();
                if !host.ends_with("gallica.bnf.fr") && host != "gallica.bnf.fr" {
                    return Err(ArkError::UnsupportedHost(host));
                }
                // Keep only the path; query and fragment are irrelevant.
                let path = url.path().to_string();
                return Self::parse_path(&path, input);
            }
        }
        // Drop query string / fragment on non-URL inputs too.
        if let Some(i) = rest.find(['?', '#']) {
            rest = &rest[..i];
        }
        Self::parse_path(rest, input)
    }

    fn parse_path(path: &str, original: &str) -> Result<Self, ArkError> {
        let path = path.trim().trim_start_matches('/');
        // Locate "ark:/" (tolerating the "ark://" typo).
        let after_ark = if let Some(i) = path.find("ark:/") {
            path[i + 5..].trim_start_matches('/')
        } else {
            path
        };
        let mut segments = after_ark.split('/').filter(|s| !s.is_empty());

        let first = segments
            .next()
            .ok_or_else(|| ArkError::Unrecognized(original.to_string()))?;

        let (naan, name_seg) = if first.chars().all(|c| c.is_ascii_digit()) {
            (
                first.to_string(),
                segments
                    .next()
                    .ok_or_else(|| ArkError::Unrecognized(original.to_string()))?,
            )
        } else {
            if path.contains("ark:/") {
                // "ark:/" present but no NAAN: malformed.
                return Err(ArkError::Unrecognized(original.to_string()));
            }
            (BNF_NAAN.to_string(), first)
        };
        if naan != BNF_NAAN {
            return Err(ArkError::UnsupportedNaan(naan));
        }

        // The name may carry a dotted suffix (e.g. "bpt6k961287k.texteBrut").
        let (name, dotted) = match name_seg.split_once('.') {
            Some((n, suffix)) => (n, Some(suffix)),
            None => (name_seg, None),
        };
        validate_name(name)?;

        let remaining: Vec<&str> = segments.collect();
        let qualifier = match (dotted, remaining.is_empty()) {
            (Some(s), true) => Some(format!(".{s}")),
            (Some(s), false) => Some(format!(".{s}/{}", remaining.join("/"))),
            (None, true) => None,
            (None, false) => Some(remaining.join("/")),
        };

        Ok(Ark {
            naan,
            name: name.to_string(),
            qualifier,
        })
    }

    /// Name assigning authority number (`12148` for the BnF).
    pub fn naan(&self) -> &str {
        &self.naan
    }

    /// The ARK name, e.g. `bpt6k961287k`.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The qualifier that followed the name in the input, if any
    /// (e.g. `date`, `f3.item`, `.texteBrut`).
    pub fn qualifier(&self) -> Option<&str> {
        self.qualifier.as_deref()
    }

    /// Returns a copy of this ARK with the qualifier removed.
    pub fn without_qualifier(&self) -> Ark {
        Ark {
            naan: self.naan.clone(),
            name: self.name.clone(),
            qualifier: None,
        }
    }

    /// Returns a copy of this ARK with the given qualifier.
    pub fn with_qualifier(&self, qualifier: impl Into<String>) -> Ark {
        Ark {
            naan: self.naan.clone(),
            name: self.name.clone(),
            qualifier: Some(qualifier.into()),
        }
    }

    /// The kind of resource this ARK designates, inferred from its name.
    pub fn kind(&self) -> ArkKind {
        if self.name.starts_with("cb") {
            ArkKind::Periodical
        } else {
            ArkKind::Document
        }
    }

    /// `true` if this ARK designates a periodical title.
    pub fn is_periodical(&self) -> bool {
        self.kind() == ArkKind::Periodical
    }

    /// The page number carried by an `fN` qualifier, if any
    /// (e.g. `f3.item` gives `Some(3)`).
    pub fn page(&self) -> Option<u32> {
        let q = self.qualifier.as_deref()?;
        let q = q.strip_prefix('f')?;
        let digits: String = q.chars().take_while(|c| c.is_ascii_digit()).collect();
        if digits.is_empty() {
            return None;
        }
        // Reject "f5n2" (a page range) as a single page.
        if q[digits.len()..].starts_with('n') {
            return None;
        }
        digits.parse().ok()
    }

    /// The canonical identifier `ark:/12148/<name>` (without qualifier).
    pub fn id(&self) -> String {
        format!("ark:/{}/{}", self.naan, self.name)
    }

    /// The identifier used by the `Issues` service: `ark:/12148/<name>/date`.
    pub fn issues_id(&self) -> String {
        format!("ark:/{}/{}/date", self.naan, self.name)
    }

    /// The Gallica landing page URL for this document (qualifier dropped).
    pub fn gallica_url(&self) -> Url {
        let mut url = Url::parse(GALLICA_BASE_URL).expect("constant URL");
        url.set_path(&format!("/{}", self.id()));
        url
    }
}

impl fmt::Display for Ark {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ark:/{}/{}", self.naan, self.name)?;
        match &self.qualifier {
            Some(q) if q.starts_with('.') => write!(f, "{q}"),
            Some(q) => write!(f, "/{q}"),
            None => Ok(()),
        }
    }
}

impl fmt::Debug for Ark {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Ark({self})")
    }
}

impl FromStr for Ark {
    type Err = ArkError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ark::parse(s)
    }
}

impl TryFrom<&str> for Ark {
    type Error = ArkError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Ark::parse(value)
    }
}

impl TryFrom<String> for Ark {
    type Error = ArkError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Ark::parse(&value)
    }
}

fn validate_name(name: &str) -> Result<(), ArkError> {
    if name.len() < 3 || !name.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Err(ArkError::InvalidName(name.to_string()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_bare_name() {
        let ark = Ark::parse("bpt6k961287k").unwrap();
        assert_eq!(ark.name(), "bpt6k961287k");
        assert_eq!(ark.naan(), "12148");
        assert_eq!(ark.qualifier(), None);
        assert_eq!(ark.kind(), ArkKind::Document);
        assert_eq!(ark.to_string(), "ark:/12148/bpt6k961287k");
    }

    #[test]
    fn parses_identifier_and_naan_name() {
        assert_eq!(
            Ark::parse("ark:/12148/bpt6k961287k").unwrap().id(),
            "ark:/12148/bpt6k961287k"
        );
        assert_eq!(
            Ark::parse("12148/bpt6k961287k").unwrap().id(),
            "ark:/12148/bpt6k961287k"
        );
        assert_eq!(
            Ark::parse("ark://12148/bpt6k961287k").unwrap().id(),
            "ark:/12148/bpt6k961287k"
        );
    }

    #[test]
    fn parses_periodical_url() {
        let ark = Ark::parse("https://gallica.bnf.fr/ark:/12148/cb327110547/").unwrap();
        assert_eq!(ark.name(), "cb327110547");
        assert_eq!(ark.kind(), ArkKind::Periodical);
        assert!(ark.is_periodical());
        assert_eq!(ark.qualifier(), None);
        assert_eq!(ark.issues_id(), "ark:/12148/cb327110547/date");

        let ark = Ark::parse("https://gallica.bnf.fr/ark:/12148/cb327110547/date").unwrap();
        assert_eq!(ark.qualifier(), Some("date"));
        assert_eq!(ark.to_string(), "ark:/12148/cb327110547/date");
    }

    #[test]
    fn parses_document_url_with_qualifiers() {
        let ark =
            Ark::parse("https://gallica.bnf.fr/ark:/12148/bpt6k961287k/f3.item.zoom?rk=21459;2")
                .unwrap();
        assert_eq!(ark.name(), "bpt6k961287k");
        assert_eq!(ark.qualifier(), Some("f3.item.zoom"));
        assert_eq!(ark.page(), Some(3));

        let ark = Ark::parse("http://gallica.bnf.fr/ark:/12148/bpt6k961287k.texteBrut").unwrap();
        assert_eq!(ark.name(), "bpt6k961287k");
        assert_eq!(ark.qualifier(), Some(".texteBrut"));
        assert_eq!(ark.to_string(), "ark:/12148/bpt6k961287k.texteBrut");
        assert_eq!(ark.page(), None);

        let ark = Ark::parse("ark:/12148/bpt6k961287k/f5n2.texteBrut").unwrap();
        assert_eq!(ark.qualifier(), Some("f5n2.texteBrut"));
        assert_eq!(ark.page(), None);
    }

    #[test]
    fn parses_iiif_url() {
        let ark = Ark::parse(
            "https://gallica.bnf.fr/iiif/ark:/12148/btv1b90017179/f15/0,1900,2400,1200/full/0/native.jpg",
        )
        .unwrap();
        assert_eq!(ark.name(), "btv1b90017179");
        assert_eq!(ark.page(), Some(15));
    }

    #[test]
    fn rejects_garbage() {
        assert_eq!(Ark::parse("   "), Err(ArkError::Empty));
        assert!(matches!(
            Ark::parse("https://example.com/ark:/12148/bpt6k961287k"),
            Err(ArkError::UnsupportedHost(_))
        ));
        assert!(matches!(
            Ark::parse("ark:/99999/bpt6k961287k"),
            Err(ArkError::UnsupportedNaan(_))
        ));
        assert!(matches!(
            Ark::parse("ark:/12148/"),
            Err(ArkError::Unrecognized(_))
        ));
        assert!(matches!(
            Ark::parse("ark:/12148/bad name"),
            Err(ArkError::InvalidName(_))
        ));
        assert!(matches!(
            Ark::parse("ark:/bpt6k961287k"),
            Err(ArkError::Unrecognized(_))
        ));
    }

    #[test]
    fn display_roundtrip() {
        for s in [
            "ark:/12148/bpt6k961287k",
            "ark:/12148/cb327110547/date",
            "ark:/12148/bpt6k961287k/f3.item",
            "ark:/12148/bpt6k961287k.texteBrut",
        ] {
            assert_eq!(Ark::parse(s).unwrap().to_string(), s);
        }
    }
}

#[cfg(test)]
mod url_tests {
    use super::*;

    #[test]
    fn gallica_url_keeps_scheme() {
        let ark = Ark::parse("ark:/12148/bpt6k961287k/f3").unwrap();
        assert_eq!(
            ark.gallica_url().as_str(),
            "https://gallica.bnf.fr/ark:/12148/bpt6k961287k"
        );
    }
}

impl serde::Serialize for Ark {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> serde::Deserialize<'de> for Ark {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        Ark::parse(&s).map_err(serde::de::Error::custom)
    }
}
