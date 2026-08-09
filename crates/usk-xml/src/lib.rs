//! usk-xml — a pull XML reader for OOXML parts (docs/24).
//!
//! > *XML: DTD/external entities disabled, depth/node caps.*
//!
//! Both halves of that sentence are enforced structurally rather than
//! configured: there is **no code here that resolves a DTD or an external
//! entity**, so the classic XXE and billion-laughs attacks are not switched
//! off, they are unimplemented. A `<!DOCTYPE` declaration is refused outright
//! rather than skipped, because a document that contains one is asking for a
//! feature this parser does not have and silently ignoring the request is how
//! a parser ends up disagreeing with the one that wrote the file.
//!
//! Only the five predefined entities and numeric character references are
//! expanded, and neither can nest, so entity expansion is O(1) per reference
//! and unbounded expansion is unreachable.
//!
//! # Scope
//! Enough XML to read OOXML, and no more: elements, attributes, text, CDATA,
//! comments and processing instructions. **Namespaces are not resolved** —
//! `<x:c r="A1">` yields the name `x:c` verbatim, and the XLSX reader matches on
//! the local part. OOXML's prefixes are conventional but not guaranteed, so the
//! reader compares local names rather than pretending prefixes are stable.

#![no_std]
extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;

/// Nesting depth. OOXML's deepest legitimate part is a handful of levels; this
/// is generous by two orders of magnitude and still bounded.
pub const MAX_DEPTH: usize = 256;
/// Attributes on one element.
pub const MAX_ATTRIBUTES: usize = 256;
/// Bytes in one element or attribute name.
pub const MAX_NAME_BYTES: usize = 1024;
/// Bytes in one text node or attribute value. Excel's cell limit is 32,767
/// characters; shared strings can hold rich text, so this is wider.
pub const MAX_TEXT_BYTES: usize = 1 << 20;

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum XmlError {
    /// The document ended inside a construct.
    Truncated {
        at: usize,
    },
    /// A `<!DOCTYPE` declaration. Refused, not ignored — see the module docs.
    DoctypeRefused {
        at: usize,
    },
    /// An entity reference other than the five predefined ones or a numeric
    /// character reference. There is no entity table to look it up in, and
    /// guessing would be worse than failing.
    UnknownEntity {
        at: usize,
        name: String,
    },
    /// A numeric character reference outside the Unicode scalar range.
    BadCharacterReference {
        at: usize,
    },
    /// A closing tag that does not match the open one.
    MismatchedTag {
        at: usize,
        expected: String,
        found: String,
    },
    /// A cap was exceeded.
    CapExceeded {
        at: usize,
        cap: &'static str,
    },
    Malformed {
        at: usize,
        what: &'static str,
    },
}

/// An element's name and attributes.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Element {
    pub name: String,
    pub attributes: Vec<(String, String)>,
    /// `<a/>` rather than `<a>`. A self-closing element still emits a matching
    /// [`Event::End`], so a consumer never needs to special-case it.
    pub self_closing: bool,
}

impl Element {
    /// The name without its namespace prefix — `x:c` becomes `c`.
    pub fn local_name(&self) -> &str {
        local(&self.name)
    }

    /// An attribute by local name, prefix-insensitive for the same reason
    /// [`Element::local_name`] exists.
    pub fn attribute(&self, name: &str) -> Option<&str> {
        self.attributes
            .iter()
            .find(|(key, _)| local(key) == name)
            .map(|(_, value)| value.as_str())
    }
}

/// The local part of a possibly-prefixed name.
pub fn local(name: &str) -> &str {
    match name.rfind(':') {
        Some(at) => &name[at + 1..],
        None => name,
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Event {
    Start(Element),
    End(String),
    /// Character data, with entities already expanded. Emitted only when it
    /// contains something other than whitespace *or* the caller asked for
    /// whitespace — OOXML uses `xml:space="preserve"`, and dropping whitespace
    /// unconditionally would silently trim cell text.
    Text(String),
}

/// A pull reader over one XML document.
pub struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
    open: Vec<String>,
    /// Emitted after a self-closing tag, so `<a/>` produces Start then End.
    pending_end: Option<String>,
    failed: bool,
}

impl<'a> Reader<'a> {
    pub fn new(bytes: &'a [u8]) -> Reader<'a> {
        // A UTF-8 BOM is metadata; OOXML parts routinely carry one.
        let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
        Reader {
            bytes,
            at: 0,
            open: Vec::new(),
            pending_end: None,
            failed: false,
        }
    }

    pub fn depth(&self) -> usize {
        self.open.len()
    }

    /// The next event, or `None` at the end of the document.
    ///
    /// Once an error is returned the reader is spent and every later call
    /// yields `None`: continuing after a structural failure would mean
    /// reporting events from a document we have already said we cannot read.
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> Option<Result<Event, XmlError>> {
        if self.failed {
            return None;
        }
        match self.step() {
            Ok(Some(event)) => Some(Ok(event)),
            Ok(None) => None,
            Err(err) => {
                self.failed = true;
                Some(Err(err))
            }
        }
    }

    fn step(&mut self) -> Result<Option<Event>, XmlError> {
        if let Some(name) = self.pending_end.take() {
            self.open.pop();
            return Ok(Some(Event::End(name)));
        }
        loop {
            if self.at >= self.bytes.len() {
                // An element left open at the end of the document is a
                // *truncation*, and saying so matters: a worksheet part cut
                // short would otherwise look like a complete sheet with fewer
                // rows, which is the silent partial read docs/16 forbids at the
                // other boundary.
                if !self.open.is_empty() {
                    return Err(XmlError::Truncated { at: self.at });
                }
                return Ok(None);
            }
            if self.bytes[self.at] != b'<' {
                let text = self.text()?;
                if text.is_empty() {
                    continue;
                }
                return Ok(Some(Event::Text(text)));
            }
            match self.markup()? {
                Some(event) => return Ok(Some(event)),
                // Comments, PIs and the XML declaration produce no event.
                None => continue,
            }
        }
    }

    /// Character data up to the next `<`.
    fn text(&mut self) -> Result<String, XmlError> {
        let start = self.at;
        while self.at < self.bytes.len() && self.bytes[self.at] != b'<' {
            self.at += 1;
        }
        let raw = &self.bytes[start..self.at];
        if raw.len() > MAX_TEXT_BYTES {
            return Err(XmlError::CapExceeded {
                at: start,
                cap: "MAX_TEXT_BYTES",
            });
        }
        let text = unescape(raw, start)?;
        if text
            .bytes()
            .all(|b| matches!(b, b' ' | b'\t' | b'\r' | b'\n'))
        {
            return Ok(String::new());
        }
        Ok(text)
    }

    /// Anything beginning `<`.
    fn markup(&mut self) -> Result<Option<Event>, XmlError> {
        let start = self.at;
        let rest = &self.bytes[self.at..];

        if rest.starts_with(b"<!--") {
            return self.skip_to(b"-->", start).map(|()| None);
        }
        if rest.starts_with(b"<![CDATA[") {
            self.at += b"<![CDATA[".len();
            let body_start = self.at;
            let end =
                find(&self.bytes[self.at..], b"]]>").ok_or(XmlError::Truncated { at: start })?;
            let body = &self.bytes[body_start..body_start + end];
            self.at = body_start + end + 3;
            // CDATA is literal: no entity expansion, by definition.
            let text = core::str::from_utf8(body)
                .map_err(|_| XmlError::Malformed {
                    at: body_start,
                    what: "CDATA is not UTF-8",
                })?
                .into();
            return Ok(Some(Event::Text(text)));
        }
        if rest.starts_with(b"<?") {
            return self.skip_to(b"?>", start).map(|()| None);
        }
        if rest.starts_with(b"<!DOCTYPE") || rest.starts_with(b"<!ENTITY") {
            // Refused rather than skipped. A document carrying a DTD is asking
            // for a feature this parser does not have, and quietly ignoring the
            // request is how a parser ends up disagreeing with the writer.
            return Err(XmlError::DoctypeRefused { at: start });
        }
        if rest.starts_with(b"</") {
            self.at += 2;
            let name = self.name()?;
            self.skip_space();
            if self.byte() != Some(b'>') {
                return Err(XmlError::Malformed {
                    at: start,
                    what: "unterminated end tag",
                });
            }
            self.at += 1;
            match self.open.pop() {
                Some(expected) if expected == name => Ok(Some(Event::End(name))),
                Some(expected) => Err(XmlError::MismatchedTag {
                    at: start,
                    expected,
                    found: name,
                }),
                None => Err(XmlError::MismatchedTag {
                    at: start,
                    expected: String::new(),
                    found: name,
                }),
            }
        } else {
            self.start_tag(start).map(Some)
        }
    }

    fn start_tag(&mut self, start: usize) -> Result<Event, XmlError> {
        self.at += 1; // '<'
        let name = self.name()?;
        let mut element = Element {
            name,
            attributes: Vec::new(),
            self_closing: false,
        };

        loop {
            self.skip_space();
            match self.byte() {
                None => return Err(XmlError::Truncated { at: start }),
                Some(b'>') => {
                    self.at += 1;
                    break;
                }
                Some(b'/') => {
                    self.at += 1;
                    if self.byte() != Some(b'>') {
                        return Err(XmlError::Malformed {
                            at: start,
                            what: "expected '>' after '/'",
                        });
                    }
                    self.at += 1;
                    element.self_closing = true;
                    break;
                }
                Some(_) => {
                    if element.attributes.len() >= MAX_ATTRIBUTES {
                        return Err(XmlError::CapExceeded {
                            at: start,
                            cap: "MAX_ATTRIBUTES",
                        });
                    }
                    let key = self.name()?;
                    self.skip_space();
                    if self.byte() != Some(b'=') {
                        return Err(XmlError::Malformed {
                            at: self.at,
                            what: "attribute without a value",
                        });
                    }
                    self.at += 1;
                    self.skip_space();
                    let value = self.attribute_value()?;
                    element.attributes.push((key, value));
                }
            }
        }

        if self.open.len() >= MAX_DEPTH {
            return Err(XmlError::CapExceeded {
                at: start,
                cap: "MAX_DEPTH",
            });
        }
        self.open.push(element.name.clone());
        if element.self_closing {
            // The matching End is emitted next, so a consumer never has to
            // special-case `<a/>` against `<a></a>`.
            self.pending_end = Some(element.name.clone());
        }
        Ok(Event::Start(element))
    }

    fn attribute_value(&mut self) -> Result<String, XmlError> {
        let quote = self.byte().ok_or(XmlError::Truncated { at: self.at })?;
        if quote != b'"' && quote != b'\'' {
            return Err(XmlError::Malformed {
                at: self.at,
                what: "unquoted attribute value",
            });
        }
        self.at += 1;
        let start = self.at;
        while self.at < self.bytes.len() && self.bytes[self.at] != quote {
            self.at += 1;
        }
        if self.at >= self.bytes.len() {
            return Err(XmlError::Truncated { at: start });
        }
        let raw = &self.bytes[start..self.at];
        if raw.len() > MAX_TEXT_BYTES {
            return Err(XmlError::CapExceeded {
                at: start,
                cap: "MAX_TEXT_BYTES",
            });
        }
        self.at += 1; // closing quote
        unescape(raw, start)
    }

    fn name(&mut self) -> Result<String, XmlError> {
        let start = self.at;
        while let Some(byte) = self.byte() {
            if byte.is_ascii_whitespace() || matches!(byte, b'>' | b'/' | b'=') {
                break;
            }
            self.at += 1;
        }
        if self.at == start {
            return Err(XmlError::Malformed {
                at: start,
                what: "empty name",
            });
        }
        if self.at - start > MAX_NAME_BYTES {
            return Err(XmlError::CapExceeded {
                at: start,
                cap: "MAX_NAME_BYTES",
            });
        }
        core::str::from_utf8(&self.bytes[start..self.at])
            .map(String::from)
            .map_err(|_| XmlError::Malformed {
                at: start,
                what: "name is not UTF-8",
            })
    }

    fn skip_to(&mut self, terminator: &[u8], start: usize) -> Result<(), XmlError> {
        let from = self.at + 2;
        let found =
            find(&self.bytes[from..], terminator).ok_or(XmlError::Truncated { at: start })?;
        self.at = from + found + terminator.len();
        Ok(())
    }

    fn skip_space(&mut self) {
        while self.byte().is_some_and(|b| b.is_ascii_whitespace()) {
            self.at += 1;
        }
    }

    fn byte(&self) -> Option<u8> {
        self.bytes.get(self.at).copied()
    }
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    (0..=haystack.len() - needle.len()).find(|&i| &haystack[i..i + needle.len()] == needle)
}

/// Expands the five predefined entities and numeric character references.
///
/// Nothing else, because there is no entity table: an unknown entity is an
/// error rather than a lookup, which is what makes billion-laughs unreachable
/// rather than merely bounded.
fn unescape(raw: &[u8], offset: usize) -> Result<String, XmlError> {
    let text = core::str::from_utf8(raw).map_err(|_| XmlError::Malformed {
        at: offset,
        what: "text is not UTF-8",
    })?;
    if !text.contains('&') {
        return Ok(String::from(text));
    }

    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        let tail = &rest[amp + 1..];
        let end = tail.find(';').ok_or(XmlError::UnknownEntity {
            at: offset + amp,
            name: String::from(&tail[..tail.len().min(16)]),
        })?;
        let name = &tail[..end];
        match name {
            "lt" => out.push('<'),
            "gt" => out.push('>'),
            "amp" => out.push('&'),
            "quot" => out.push('"'),
            "apos" => out.push('\''),
            _ => {
                let scalar = character_reference(name).ok_or_else(|| XmlError::UnknownEntity {
                    at: offset + amp,
                    name: String::from(name),
                })?;
                out.push(
                    char::from_u32(scalar)
                        .ok_or(XmlError::BadCharacterReference { at: offset + amp })?,
                );
            }
        }
        rest = &tail[end + 1..];
    }
    out.push_str(rest);
    Ok(out)
}

fn character_reference(name: &str) -> Option<u32> {
    let digits = name.strip_prefix('#')?;
    match digits.strip_prefix(['x', 'X']) {
        Some(hex) => u32::from_str_radix(hex, 16).ok(),
        None => digits.parse::<u32>().ok(),
    }
}
