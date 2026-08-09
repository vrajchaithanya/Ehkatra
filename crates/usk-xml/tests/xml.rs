//! The XML reader against OOXML shapes and against the attacks docs/24 names.

use usk_xml::{local, Element, Event, Reader, XmlError, MAX_DEPTH};

fn events(xml: &str) -> Vec<Event> {
    let mut reader = Reader::new(xml.as_bytes());
    let mut out = Vec::new();
    while let Some(event) = reader.next() {
        out.push(event.unwrap_or_else(|e| panic!("{xml:?}: {e:?}")));
    }
    out
}

fn first_error(xml: &str) -> XmlError {
    let mut reader = Reader::new(xml.as_bytes());
    while let Some(event) = reader.next() {
        if let Err(err) = event {
            return err;
        }
    }
    panic!("{xml:?} was expected to fail");
}

fn start(event: &Event) -> &Element {
    match event {
        Event::Start(element) => element,
        other => panic!("expected a start tag, got {other:?}"),
    }
}

// ------------------------------------------------------------ the grammar

#[test]
fn a_worksheet_fragment_parses_the_way_ooxml_writes_it() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
  <sheetData>
    <row r="1"><c r="A1" t="s"><v>0</v></c><c r="B1"><v>1.5</v></c></row>
    <row r="2"><c r="A2" s="3"/></row>
  </sheetData>
</worksheet>"#;
    let events = events(xml);

    let cells: Vec<&Element> = events
        .iter()
        .filter_map(|e| match e {
            Event::Start(el) if el.local_name() == "c" => Some(el),
            _ => None,
        })
        .collect();
    assert_eq!(cells.len(), 3);
    assert_eq!(cells[0].attribute("r"), Some("A1"));
    assert_eq!(cells[0].attribute("t"), Some("s"));
    assert_eq!(cells[1].attribute("t"), None);
    assert!(cells[2].self_closing);

    let texts: Vec<&String> = events
        .iter()
        .filter_map(|e| match e {
            Event::Text(t) => Some(t),
            _ => None,
        })
        .collect();
    assert_eq!(texts, vec!["0", "1.5"]);
}

/// `<a/>` and `<a></a>` must be indistinguishable to a consumer, or every
/// consumer grows the same special case.
#[test]
fn a_self_closing_element_still_emits_its_end() {
    assert_eq!(
        events("<a/>"),
        vec![
            Event::Start(Element {
                name: String::from("a"),
                attributes: Vec::new(),
                self_closing: true
            }),
            Event::End(String::from("a")),
        ]
    );
    let paired = events("<a></a>");
    assert_eq!(paired.len(), 2);
    assert!(matches!(paired[1], Event::End(_)));
}

/// OOXML prefixes are conventional, not guaranteed, so the reader compares
/// local names rather than pretending `x:` is stable.
#[test]
fn namespace_prefixes_are_kept_but_matching_is_by_local_name() {
    let parsed = events(r#"<x:c x:r="A1" r="B2"/>"#);
    let element = start(&parsed[0]);
    assert_eq!(element.name, "x:c", "the prefix is preserved verbatim");
    assert_eq!(element.local_name(), "c");
    assert_eq!(
        element.attribute("r"),
        Some("A1"),
        "the first matching local name wins"
    );
    assert_eq!(local("a:b:c"), "c");
    assert_eq!(local("plain"), "plain");
}

#[test]
fn entities_and_character_references_expand() {
    assert_eq!(
        events("<t>&lt;&gt;&amp;&quot;&apos;</t>")[1],
        Event::Text(String::from("<>&\"'"))
    );
    assert_eq!(
        events("<t>&#65;&#x1F600;</t>")[1],
        Event::Text(String::from("A\u{1F600}"))
    );
    let attributed = events(r#"<c r="A&amp;1"/>"#);
    assert_eq!(start(&attributed[0]).attribute("r"), Some("A&1"));
}

/// OOXML uses `xml:space="preserve"`, so whitespace inside a text node is data.
/// Whitespace *between* elements is not, and dropping it keeps consumers from
/// having to filter every indentation newline.
#[test]
fn whitespace_between_elements_is_dropped_but_inside_text_is_kept() {
    assert_eq!(events("<a>\n  <b/>\n</a>").len(), 4, "no text events");
    assert_eq!(
        events("<t xml:space=\"preserve\"> padded </t>")[1],
        Event::Text(String::from(" padded "))
    );
}

#[test]
fn comments_cdata_and_processing_instructions_are_handled() {
    assert_eq!(
        events("<a><!-- <b/> --></a>").len(),
        2,
        "a comment is not markup"
    );
    assert_eq!(
        events("<t><![CDATA[ raw & < > ]]></t>")[1],
        Event::Text(String::from(" raw & < > ")),
        "CDATA is literal: no entity expansion, by definition"
    );
    assert_eq!(events("<?pi data?><a/>").len(), 2);
}

// -------------------------------------------------- the attacks docs/24 names

/// **XXE and billion-laughs are unimplemented, not disabled.** There is no
/// entity table and no DTD parser, so a document asking for one is refused
/// rather than silently reinterpreted.
#[test]
fn a_doctype_is_refused_rather_than_skipped() {
    let xxe = r#"<?xml version="1.0"?>
<!DOCTYPE foo [<!ENTITY xxe SYSTEM "file:///etc/passwd">]>
<foo>&xxe;</foo>"#;
    assert!(matches!(first_error(xxe), XmlError::DoctypeRefused { .. }));

    let lol =
        r#"<!DOCTYPE lolz [<!ENTITY lol "lol"><!ENTITY lol2 "&lol;&lol;">]><lolz>&lol2;</lolz>"#;
    assert!(matches!(first_error(lol), XmlError::DoctypeRefused { .. }));
}

/// Even without a DTD, an undefined entity must be an error rather than a
/// guess — there is nowhere to look it up, and inventing an expansion would be
/// worse than failing.
#[test]
fn an_unknown_entity_is_an_error_not_a_guess() {
    match first_error("<t>&xxe;</t>") {
        XmlError::UnknownEntity { name, .. } => assert_eq!(name, "xxe"),
        other => panic!("expected UnknownEntity, got {other:?}"),
    }
    assert!(matches!(
        first_error("<t>&#xFFFFFFFF;</t>"),
        XmlError::UnknownEntity { .. } | XmlError::BadCharacterReference { .. }
    ));
    // A surrogate is not a scalar value.
    assert!(matches!(
        first_error("<t>&#xD800;</t>"),
        XmlError::BadCharacterReference { .. }
    ));
}

#[test]
fn nesting_is_bounded() {
    let deep = "<a>".repeat(MAX_DEPTH + 5) + &"</a>".repeat(MAX_DEPTH + 5);
    assert!(matches!(
        first_error(&deep),
        XmlError::CapExceeded {
            cap: "MAX_DEPTH",
            ..
        }
    ));
    let fine = "<a>".repeat(MAX_DEPTH - 1) + &"</a>".repeat(MAX_DEPTH - 1);
    let mut reader = Reader::new(fine.as_bytes());
    while let Some(event) = reader.next() {
        assert!(event.is_ok(), "just inside the bound must parse");
    }
}

#[test]
fn structural_defects_are_named() {
    assert!(matches!(
        first_error("<a></b>"),
        XmlError::MismatchedTag { .. }
    ));
    // An element left open at the end of the document is a truncation, not a
    // complete document with fewer children.
    assert!(matches!(first_error("<a>"), XmlError::Truncated { .. }));
    assert!(matches!(first_error("<a><b/>"), XmlError::Truncated { .. }));
    assert!(matches!(
        first_error("<a b=unquoted/>"),
        XmlError::Malformed { .. }
    ));
    assert!(matches!(first_error("<a b/>"), XmlError::Malformed { .. }));
    assert!(matches!(
        first_error("</a>"),
        XmlError::MismatchedTag { .. }
    ));
}

/// Once the reader has said it cannot read a document it must stop, not carry
/// on reporting events from a structure it has already rejected.
#[test]
fn a_failed_reader_is_spent() {
    let mut reader = Reader::new(b"<a></b><c/>");
    let mut errors = 0;
    let mut after = 0;
    while let Some(event) = reader.next() {
        match event {
            Err(_) => errors += 1,
            Ok(_) if errors > 0 => after += 1,
            Ok(_) => {}
        }
    }
    assert_eq!(errors, 1);
    assert_eq!(after, 0, "nothing may follow a structural failure");
}

/// Totality over arbitrary bytes, including every truncation of a real
/// fragment: a short read is the ordinary case for a streamed part.
#[test]
fn no_input_can_make_the_reader_panic() {
    let document = r#"<?xml version="1.0"?><a x="1"><b/><!--c--><![CDATA[d]]>&amp;</a>"#;
    for cut in 0..document.len() {
        let mut reader = Reader::new(&document.as_bytes()[..cut]);
        while let Some(_event) = reader.next() {}
    }

    let mut seed = 0xC0FF_EE00_1234_5678u64;
    let mut next = move || {
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        seed >> 33
    };
    let alphabet = b"<>/=\"'&; abc:!-[]?xml0123DOCTYPEACD";
    for _ in 0..20_000 {
        let len = (next() % 60) as usize + 1;
        let bytes: Vec<u8> = (0..len)
            .map(|_| alphabet[(next() as usize) % alphabet.len()])
            .collect();
        let mut reader = Reader::new(&bytes);
        while let Some(_event) = reader.next() {}
    }
}
