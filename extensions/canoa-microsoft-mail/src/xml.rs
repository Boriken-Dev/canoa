//! The little XML the Exchange web services speak. A reader into a tree of elements with their attributes and text, and the
//! escaping for what is written.
//!
//! Namespaces are reduced to local names (`t:Subject` is `Subject`): EWS never reuses a local name for two things in one place.
//! A document type declaration is refused, so no entity can be defined or expanded; only the five predefined entities and
//! character references are read.

use core::fmt;

/// The deepest nesting read.
const MAX_DEPTH: usize = 128;

/// An element: its local name, attributes, children and the text directly inside it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Element {
    pub(crate) name: String,
    pub(crate) attrs: Vec<(String, String)>,
    pub(crate) children: Vec<Element>,
    pub(crate) text: String,
}

impl Element {
    pub(crate) fn attr(&self, name: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }

    pub(crate) fn child(&self, name: &str) -> Option<&Element> {
        self.children.iter().find(|child| child.name == name)
    }

    pub(crate) fn children_named<'a>(
        &'a self,
        name: &'a str,
    ) -> impl Iterator<Item = &'a Element> + 'a {
        self.children.iter().filter(move |child| child.name == name)
    }

    /// The element at `path`, names joined with `/`, from here.
    pub(crate) fn path(&self, path: &str) -> Option<&Element> {
        path.split('/')
            .try_fold(self, |element, name| element.child(name))
    }

    /// The text of the element at `path`, trimmed; `None` when it is absent or empty.
    pub(crate) fn text_at(&self, path: &str) -> Option<&str> {
        self.path(path)
            .map(|element| element.text.trim())
            .filter(|text| !text.is_empty())
    }

    /// Every element named `name` anywhere below, in document order.
    pub(crate) fn descendants<'a>(&'a self, name: &str) -> Vec<&'a Element> {
        let mut found = Vec::new();
        let mut stack: Vec<&Element> = self.children.iter().rev().collect();
        while let Some(element) = stack.pop() {
            if element.name == name {
                found.push(element);
            }
            stack.extend(element.children.iter().rev());
        }
        found
    }

    /// Every element anywhere below with an attribute `name` equal to `value`; with an empty `value`, every one that has it.
    pub(crate) fn descendants_with_attr(&self, name: &str, value: &str) -> Vec<&Element> {
        let mut found = Vec::new();
        let mut stack: Vec<&Element> = self.children.iter().rev().collect();
        while let Some(element) = stack.pop() {
            if element
                .attr(name)
                .is_some_and(|have| value.is_empty() || have == value)
            {
                found.push(element);
            }
            stack.extend(element.children.iter().rev());
        }
        found
    }
}

/// Why a document could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct XmlError(pub(crate) String);

impl fmt::Display for XmlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "the answer is not XML this reader takes: {}", self.0)
    }
}

impl core::error::Error for XmlError {}

fn local(name: &str) -> &str {
    name.rsplit(':').next().unwrap_or(name)
}

/// Reads one document.
///
/// # Errors
///
/// The text is not well formed, nests too deeply, or has a document type declaration.
pub(crate) fn parse(input: &str) -> Result<Element, XmlError> {
    let fail = |why: &str| XmlError(why.to_owned());
    let mut stack: Vec<Element> = Vec::new();
    let mut root: Option<Element> = None;
    let mut rest = input;
    while !rest.is_empty() {
        if let Some(tail) = rest.strip_prefix("<!--") {
            let end = tail
                .find("-->")
                .ok_or_else(|| fail("a comment is not closed"))?;
            rest = &tail[end + 3..];
        } else if let Some(tail) = rest.strip_prefix("<![CDATA[") {
            let end = tail
                .find("]]>")
                .ok_or_else(|| fail("a CDATA is not closed"))?;
            if let Some(top) = stack.last_mut() {
                top.text.push_str(&tail[..end]);
            }
            rest = &tail[end + 3..];
        } else if let Some(tail) = rest.strip_prefix("<?") {
            let end = tail
                .find("?>")
                .ok_or_else(|| fail("a declaration is not closed"))?;
            rest = &tail[end + 2..];
        } else if rest.starts_with("<!") {
            return Err(fail("a document type declaration is refused"));
        } else if let Some(tail) = rest.strip_prefix("</") {
            let end = tail
                .find('>')
                .ok_or_else(|| fail("an end tag is not closed"))?;
            let name = local(tail[..end].trim());
            let element = stack.pop().ok_or_else(|| fail("an end tag has no start"))?;
            if element.name != name {
                return Err(fail("an end tag does not match its start"));
            }
            match stack.last_mut() {
                Some(parent) => parent.children.push(element),
                None => root = Some(element),
            }
            rest = &tail[end + 1..];
        } else if let Some(tail) = rest.strip_prefix('<') {
            let (element, empty, after) = start_tag(tail)?;
            rest = after;
            if empty {
                match stack.last_mut() {
                    Some(parent) => parent.children.push(element),
                    None => root = Some(element),
                }
            } else {
                if stack.len() >= MAX_DEPTH {
                    return Err(fail("the nesting is too deep"));
                }
                stack.push(element);
            }
        } else {
            let end = rest.find('<').unwrap_or(rest.len());
            if let Some(top) = stack.last_mut() {
                top.text.push_str(&unescape(&rest[..end]));
            }
            rest = &rest[end..];
        }
    }
    if !stack.is_empty() {
        return Err(fail("an element is not closed"));
    }
    root.ok_or_else(|| fail("there is no element"))
}

/// A start tag after its `<`: the element, whether it closes itself, and what follows.
fn start_tag(tail: &str) -> Result<(Element, bool, &str), XmlError> {
    let fail = |why: &str| XmlError(why.to_owned());
    let name_end = tail
        .find(|c: char| c.is_whitespace() || c == '/' || c == '>')
        .ok_or_else(|| fail("a start tag is not closed"))?;
    let mut element = Element {
        name: local(&tail[..name_end]).to_owned(),
        ..Element::default()
    };
    if element.name.is_empty() {
        return Err(fail("an element has no name"));
    }
    let mut rest = &tail[name_end..];
    loop {
        rest = rest.trim_start();
        if let Some(after) = rest.strip_prefix("/>") {
            return Ok((element, true, after));
        }
        if let Some(after) = rest.strip_prefix('>') {
            return Ok((element, false, after));
        }
        let eq = rest
            .find('=')
            .ok_or_else(|| fail("an attribute has no value"))?;
        let key = rest[..eq].trim();
        let value_start = rest[eq + 1..].trim_start();
        let quote = value_start
            .chars()
            .next()
            .filter(|c| *c == '"' || *c == '\'')
            .ok_or_else(|| fail("an attribute value is not quoted"))?;
        let body = &value_start[1..];
        let close = body
            .find(quote)
            .ok_or_else(|| fail("an attribute value is not closed"))?;
        if key != "xmlns" && !key.starts_with("xmlns:") {
            element
                .attrs
                .push((local(key).to_owned(), unescape(&body[..close])));
        }
        rest = &body[close + 1..];
    }
}

/// `text` with its character references and the five predefined entities replaced. Anything else is left as written.
fn unescape(text: &str) -> String {
    if !text.contains('&') {
        return text.to_owned();
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        rest = &rest[at..];
        let replaced = rest.find(';').filter(|end| *end <= 10).and_then(|end| {
            let name = &rest[1..end];
            let c = match name {
                "lt" => Some('<'),
                "gt" => Some('>'),
                "amp" => Some('&'),
                "quot" => Some('"'),
                "apos" => Some('\''),
                _ => name
                    .strip_prefix("#x")
                    .or_else(|| name.strip_prefix("#X"))
                    .and_then(|hex| u32::from_str_radix(hex, 16).ok())
                    .or_else(|| name.strip_prefix('#').and_then(|dec| dec.parse().ok()))
                    .and_then(char::from_u32),
            };
            c.map(|c| (c, end))
        });
        if let Some((c, end)) = replaced {
            out.push(c);
            rest = &rest[end + 1..];
        } else {
            out.push('&');
            rest = &rest[1..];
        }
    }
    out.push_str(rest);
    out
}

/// `text` as element content or an attribute value.
pub(crate) fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            // XML 1.0 has no character for these.
            c if (c as u32) < 0x20 && !matches!(c, '\t' | '\n' | '\r') => {}
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn namespaces_become_local_names_and_text_is_decoded() {
        let tree = parse(
            "<?xml version=\"1.0\"?><s:Envelope xmlns:s=\"u\"><s:Body a:Id=\"x&amp;y\" xmlns:a=\"v\">\
             <t:Subject>R&amp;D &lt;1&gt; &#65;&#x42;<![CDATA[ <raw>]]></t:Subject><t:Empty/></s:Body></s:Envelope>",
        )
        .expect("parses");
        assert_eq!(tree.name, "Envelope");
        let body = tree.child("Body").expect("body");
        assert_eq!(body.attr("Id"), Some("x&y"));
        assert_eq!(body.attrs.len(), 1, "xmlns declarations are not attributes");
        assert_eq!(tree.text_at("Body/Subject"), Some("R&D <1> AB <raw>"));
        assert!(tree.path("Body/Empty").is_some());
        assert_eq!(tree.text_at("Body/Empty"), None);
    }

    #[test]
    fn descendants_come_in_document_order() {
        let tree = parse("<a><b><c>1</c></b><c>2</c><d><c>3</c></d></a>").expect("parses");
        let texts: Vec<&str> = tree
            .descendants("c")
            .into_iter()
            .map(|c| c.text.as_str())
            .collect();
        assert_eq!(texts, ["1", "2", "3"]);
    }

    #[test]
    fn a_document_type_declaration_is_refused() {
        let error = parse("<!DOCTYPE a [<!ENTITY x \"y\">]><a>&x;</a>").expect_err("refused");
        assert!(error.0.contains("document type"));
    }

    #[test]
    fn malformed_documents_are_errors_not_panics() {
        for bad in [
            "",
            "<a>",
            "<a></b>",
            "</a>",
            "<a b=c/>",
            "<a b=\"c/>",
            "<a><!-- x </a>",
            "text only",
        ] {
            assert!(parse(bad).is_err(), "{bad:?} should not parse");
        }
        let deep = "<a>".repeat(MAX_DEPTH + 1);
        assert!(parse(&deep).is_err());
    }

    #[test]
    fn escape_round_trips_through_the_reader() {
        let text = "a < b && \"c\" > 'd'\u{1}";
        let tree = parse(&format!("<a>{}</a>", escape(text))).expect("parses");
        assert_eq!(tree.text, "a < b && \"c\" > 'd'");
    }

    #[test]
    fn an_unknown_entity_is_left_as_written() {
        let tree = parse("<a>&nbsp; &amp</a>").expect("parses");
        assert_eq!(tree.text, "&nbsp; &amp");
    }
}
