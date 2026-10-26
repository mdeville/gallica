//! A tiny namespace-agnostic XML tree used to read Gallica's XML answers.
//!
//! Gallica mixes several namespaces (`srw:`, `oai_dc:`, `dc:`...) and uses
//! repeated elements liberally, which makes strongly typed deserialisation
//! brittle. Instead we parse into a small tree and query it by local name.

use quick_xml::Reader;
use quick_xml::events::Event;

use crate::error::{Error, Result};

/// An XML element with its attributes and children.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Element {
    /// Local name (namespace prefix stripped).
    pub name: String,
    /// Attributes as `(local name, value)` pairs, in document order.
    pub attrs: Vec<(String, String)>,
    /// Child nodes in document order.
    pub children: Vec<Node>,
}

/// A child node: either an element or a text run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Node {
    /// A child element.
    Element(Element),
    /// A text run (entities already decoded, surrounding whitespace trimmed).
    Text(String),
}

impl Element {
    /// Parses an XML document and returns its root element.
    pub fn parse(xml: &str, context: &'static str) -> Result<Element> {
        let mut reader = Reader::from_str(xml);
        reader.config_mut().trim_text(true);
        reader.config_mut().expand_empty_elements = false;

        let mut stack: Vec<Element> = Vec::new();
        let mut root: Option<Element> = None;

        loop {
            let event = reader.read_event().map_err(|e| {
                Error::xml(context, format!("{e} at byte {}", reader.buffer_position()))
            })?;
            match event {
                Event::Start(start) => {
                    let el = Element {
                        name: local_name(start.name().as_ref()),
                        attrs: read_attrs(&start),
                        children: Vec::new(),
                    };
                    stack.push(el);
                }
                Event::Empty(start) => {
                    let el = Element {
                        name: local_name(start.name().as_ref()),
                        attrs: read_attrs(&start),
                        children: Vec::new(),
                    };
                    push_child(&mut stack, &mut root, Node::Element(el));
                }
                Event::End(_) => {
                    let el = stack
                        .pop()
                        .ok_or_else(|| Error::xml(context, "unbalanced end tag"))?;
                    push_child(&mut stack, &mut root, Node::Element(el));
                }
                Event::Text(text) => {
                    let s = match text.unescape() {
                        Ok(s) => s.into_owned(),
                        // Unknown entities (HTML ones in particular): keep raw text.
                        Err(_) => String::from_utf8_lossy(text.as_ref()).into_owned(),
                    };
                    if !s.trim().is_empty() {
                        push_child(&mut stack, &mut root, Node::Text(s));
                    }
                }
                Event::CData(cdata) => {
                    let s = String::from_utf8_lossy(cdata.as_ref()).into_owned();
                    if !s.trim().is_empty() {
                        push_child(&mut stack, &mut root, Node::Text(s));
                    }
                }
                Event::Eof => break,
                _ => {}
            }
        }

        if !stack.is_empty() {
            return Err(Error::xml(context, "unexpected end of document"));
        }
        root.ok_or_else(|| Error::xml(context, "empty document"))
    }

    /// The value of the attribute with the given local name.
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }

    /// The value of the first attribute found among the given local names.
    /// Gallica has renamed some attributes from snake_case to camelCase over
    /// time, so parsers accept both spellings.
    pub fn attr_any(&self, names: &[&str]) -> Option<&str> {
        names.iter().find_map(|n| self.attr(n))
    }

    /// Direct child elements with the given local name.
    pub fn children(&self, name: &str) -> impl Iterator<Item = &Element> {
        self.elements().filter(move |e| e.name == name)
    }

    /// All direct child elements.
    pub fn elements(&self) -> impl Iterator<Item = &Element> {
        self.children.iter().filter_map(|n| match n {
            Node::Element(e) => Some(e),
            Node::Text(_) => None,
        })
    }

    /// The first direct child element with the given local name.
    pub fn child(&self, name: &str) -> Option<&Element> {
        self.children(name).next()
    }

    /// The trimmed text of the first direct child element with the given name.
    pub fn child_text(&self, name: &str) -> Option<String> {
        self.child(name).map(|e| e.text())
    }

    /// The first descendant element (depth first) with the given local name.
    pub fn find(&self, name: &str) -> Option<&Element> {
        for e in self.elements() {
            if e.name == name {
                return Some(e);
            }
            if let Some(found) = e.find(name) {
                return Some(found);
            }
        }
        None
    }

    /// All descendant elements (depth first) with the given local name.
    pub fn find_all<'a>(&'a self, name: &'a str) -> Vec<&'a Element> {
        let mut out = Vec::new();
        self.collect_all(name, &mut out);
        out
    }

    fn collect_all<'a>(&'a self, name: &str, out: &mut Vec<&'a Element>) {
        for e in self.elements() {
            if e.name == name {
                out.push(e);
            }
            e.collect_all(name, out);
        }
    }

    /// The concatenated text of this element and all its descendants.
    pub fn text(&self) -> String {
        let mut s = String::new();
        self.append_text(&mut s);
        s.trim().to_string()
    }

    fn append_text(&self, out: &mut String) {
        for n in &self.children {
            match n {
                Node::Text(t) => {
                    if !out.is_empty()
                        && !out.ends_with(char::is_whitespace)
                        && !t.starts_with(char::is_whitespace)
                    {
                        out.push(' ');
                    }
                    out.push_str(t);
                }
                Node::Element(e) => e.append_text(out),
            }
        }
    }

    /// The text parsed as a number.
    pub fn parse_text<T: std::str::FromStr>(&self) -> Option<T> {
        self.text().parse().ok()
    }
}

fn push_child(stack: &mut [Element], root: &mut Option<Element>, node: Node) {
    match stack.last_mut() {
        Some(parent) => parent.children.push(node),
        None => {
            if let Node::Element(e) = node
                && root.is_none()
            {
                *root = Some(e);
            }
        }
    }
}

fn local_name(raw: &[u8]) -> String {
    let s = String::from_utf8_lossy(raw);
    match s.rsplit_once(':') {
        Some((_, local)) => local.to_string(),
        None => s.into_owned(),
    }
}

fn read_attrs(start: &quick_xml::events::BytesStart<'_>) -> Vec<(String, String)> {
    start
        .attributes()
        .filter_map(|a| a.ok())
        .map(|a| {
            let key = local_name(a.key.as_ref());
            let value = match a.unescape_value() {
                Ok(v) => v.into_owned(),
                Err(_) => String::from_utf8_lossy(&a.value).into_owned(),
            };
            (key, value)
        })
        .collect()
}

/// Parses a boolean the way Gallica writes them (`true`/`false`, `yes`/`no`).
pub fn parse_bool(s: &str) -> Option<bool> {
    match s.trim().to_ascii_lowercase().as_str() {
        "true" | "yes" | "1" => Some(true),
        "false" | "no" | "0" => Some(false),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_namespaced_tree() {
        let xml = r#"<?xml version="1.0"?>
            <srw:root xmlns:srw="urn:x" xmlns:dc="urn:y" count="2">
              <dc:title xml:lang="fre">Les &amp; co</dc:title>
              <dc:title>Second</dc:title>
              <empty/>
              <nested><deep>value</deep></nested>
              <mixed>a <b>bold</b> c</mixed>
            </srw:root>"#;
        let root = Element::parse(xml, "test").unwrap();
        assert_eq!(root.name, "root");
        assert_eq!(root.attr("count"), Some("2"));
        let titles: Vec<String> = root.children("title").map(|e| e.text()).collect();
        assert_eq!(titles, vec!["Les & co", "Second"]);
        assert_eq!(root.child("title").unwrap().attr("lang"), Some("fre"));
        assert!(root.child("empty").is_some());
        assert_eq!(root.find("deep").unwrap().text(), "value");
        assert_eq!(root.child("mixed").unwrap().text(), "a bold c");
        assert_eq!(root.find_all("title").len(), 2);
    }

    #[test]
    fn tolerates_html_entities() {
        let xml = "<r><t>Caf&eacute;</t></r>";
        let root = Element::parse(xml, "test").unwrap();
        assert!(root.child_text("t").unwrap().contains("eacute"));
    }

    #[test]
    fn reports_malformed() {
        assert!(Element::parse("<r><a></r>", "test").is_err());
        assert!(Element::parse("", "test").is_err());
    }
}
