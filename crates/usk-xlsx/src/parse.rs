//! Reading the parts: workbook, relationships, shared strings, styles, sheets.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use usk_types::{CellError, ErrorKind, Origin, Value};
use usk_xml::{Event, Reader};
use usk_zip::Archive;

use crate::{
    is_active_content, is_known_unmodelled, Alignment, Cell, Fidelity, FontFacet, Loss, LossReason,
    Sheet, Workbook, XlsxError,
};

/// Reads a workbook from a container's bytes.
pub fn read(bytes: &[u8]) -> Result<Workbook, XlsxError> {
    let archive = Archive::open(bytes)?;
    let mut fidelity = Fidelity {
        parts_total: archive.entries().len(),
        ..Fidelity::default()
    };

    // Classify every part before reading any of it. Active content is
    // identified and set aside *without being decompressed* — the quarantine is
    // "we did not touch it", not "we touched it carefully".
    for entry in archive.entries() {
        if entry.is_directory() {
            continue;
        }
        if is_active_content(&entry.name) {
            fidelity.quarantined.push(entry.name.clone());
        } else if crate::is_package_plumbing(&entry.name) {
            fidelity.parts_structural.push(entry.name.clone());
        } else if is_known_unmodelled(&entry.name) {
            fidelity.parts_ignored.push(entry.name.clone());
        }
    }

    let workbook_part = "xl/workbook.xml";
    let workbook_xml = archive
        .read_named(workbook_part)
        .ok_or(XlsxError::NotAWorkbook)?
        .map_err(XlsxError::Zip)?;
    fidelity.parts_read.push(workbook_part.to_string());

    let relationships = read_relationships(&archive, &mut fidelity)?;
    let shared = read_shared_strings(&archive, &mut fidelity)?;
    let formats = read_styles(&archive, &mut fidelity)?;
    let sheet_refs = read_workbook(&workbook_xml)?;

    let mut sheets = Vec::with_capacity(sheet_refs.len());
    for reference in sheet_refs {
        let part = relationships
            .iter()
            .find(|(id, _)| *id == reference.relationship_id)
            .map(|(_, target)| resolve_target(target))
            // A sheet whose relationship is missing still has a conventional
            // location. Guessing is better than dropping the sheet, and the
            // guess is Excel's own convention.
            .unwrap_or_else(|| format!("xl/worksheets/sheet{}.xml", reference.ordinal));

        let Some(part_bytes) = archive.read_named(&part) else {
            fidelity.losses.push(Loss {
                part: part.clone(),
                reference: reference.name.clone(),
                reason: LossReason::UnparseableReference,
            });
            continue;
        };
        let part_bytes = part_bytes.map_err(XlsxError::Zip)?;
        fidelity.parts_read.push(part.clone());

        let cells = read_sheet(&part_bytes, &part, &shared, &formats, &mut fidelity)?;
        fidelity.cells_read += cells.len();
        fidelity.formulas_read += cells.iter().filter(|c| c.formula.is_some()).count();
        fidelity.number_formats_resolved +=
            cells.iter().filter(|c| c.number_format.is_some()).count();
        fidelity.styles_resolved += cells.iter().filter(|c| !c.is_unformatted()).count();
        sheets.push(Sheet {
            name: reference.name,
            part,
            cells,
        });
    }

    Ok(Workbook { sheets, fidelity })
}

struct SheetRef {
    name: String,
    relationship_id: String,
    ordinal: usize,
}

fn read_workbook(bytes: &[u8]) -> Result<Vec<SheetRef>, XlsxError> {
    let mut reader = Reader::new(bytes);
    let mut sheets = Vec::new();
    while let Some(event) = reader.next() {
        match event.map_err(|e| bad("xl/workbook.xml", e))? {
            Event::Start(element) if element.local_name() == "sheet" => {
                sheets.push(SheetRef {
                    name: element.attribute("name").unwrap_or("Sheet").to_string(),
                    // `r:id` — the local name is `id`, and matching on the
                    // local part is why the prefix does not have to be `r`.
                    relationship_id: element.attribute("id").unwrap_or("").to_string(),
                    ordinal: sheets.len() + 1,
                });
            }
            _ => {}
        }
    }
    Ok(sheets)
}

fn read_relationships(
    archive: &Archive,
    fidelity: &mut Fidelity,
) -> Result<Vec<(String, String)>, XlsxError> {
    let part = "xl/_rels/workbook.xml.rels";
    let Some(bytes) = archive.read_named(part) else {
        return Ok(Vec::new());
    };
    let bytes = bytes.map_err(XlsxError::Zip)?;
    fidelity.parts_read.push(part.to_string());

    let mut reader = Reader::new(&bytes);
    let mut out = Vec::new();
    while let Some(event) = reader.next() {
        if let Event::Start(element) = event.map_err(|e| bad(part, e))? {
            if element.local_name() == "Relationship" {
                if let (Some(id), Some(target)) =
                    (element.attribute("Id"), element.attribute("Target"))
                {
                    out.push((id.to_string(), target.to_string()));
                }
            }
        }
    }
    Ok(out)
}

/// A relationship target is relative to the part's own folder, so
/// `worksheets/sheet1.xml` in `xl/_rels/` means `xl/worksheets/sheet1.xml`.
fn resolve_target(target: &str) -> String {
    let target = target.trim_start_matches('/');
    if target.starts_with("xl/") {
        return target.to_string();
    }
    format!("xl/{target}")
}

/// The shared-string table. XLSX stores repeated text once and refers to it by
/// index, so a workbook read without this is a workbook of numbers.
fn read_shared_strings(
    archive: &Archive,
    fidelity: &mut Fidelity,
) -> Result<Vec<String>, XlsxError> {
    let part = "xl/sharedStrings.xml";
    let Some(bytes) = archive.read_named(part) else {
        return Ok(Vec::new());
    };
    let bytes = bytes.map_err(XlsxError::Zip)?;
    fidelity.parts_read.push(part.to_string());

    let mut reader = Reader::new(&bytes);
    let mut strings = Vec::new();
    let mut current = String::new();
    let mut in_item = false;
    let mut in_text = false;
    while let Some(event) = reader.next() {
        match event.map_err(|e| bad(part, e))? {
            Event::Start(element) => match element.local_name() {
                "si" => {
                    in_item = true;
                    current.clear();
                }
                "t" => in_text = true,
                _ => {}
            },
            Event::End(name) => match usk_xml::local(&name) {
                "si" if in_item => {
                    in_item = false;
                    strings.push(core::mem::take(&mut current));
                }
                "t" => in_text = false,
                _ => {}
            },
            // Rich text splits one string across several `<t>` runs; they
            // concatenate, which is what Excel displays and what `LEN` counts.
            Event::Text(text) if in_item && in_text => current.push_str(&text),
            Event::Text(_) => {}
        }
    }
    Ok(strings)
}

/// One `cellXfs` entry, resolved to the facets ADR-041 models.
///
/// This is the flyweight table XLSX has always had: a cell names an index, the
/// entry names a `numFmtId`, a `fontId` and a `fillId`, and each of those is an
/// index into its own table. Reading it is the reason a whole formatted column
/// costs one entry in the file rather than a million.
#[derive(Clone, Default, PartialEq, Debug)]
pub(crate) struct CellXf {
    pub number_format: String,
    pub font: Option<FontFacet>,
    pub fill: Option<u32>,
    pub alignment: Option<Alignment>,
}

/// `styles.xml` → the facets each `cellXfs` index resolves to.
///
/// `fontId` 0 and `fillId` 0/1 resolve to `None` on purpose: index 0 is the
/// workbook default font and 0/1 are the `none`/`gray125` fills every styles
/// part is required to carry. Treating those as facets would mark every cell in
/// every workbook as formatted, and a facet is by definition what *differs*
/// from the default.
fn read_styles(archive: &Archive, fidelity: &mut Fidelity) -> Result<Vec<CellXf>, XlsxError> {
    let part = "xl/styles.xml";
    let Some(bytes) = archive.read_named(part) else {
        return Ok(Vec::new());
    };
    let bytes = bytes.map_err(XlsxError::Zip)?;
    fidelity.parts_read.push(part.to_string());

    let mut reader = Reader::new(&bytes);
    let mut custom: Vec<(u32, String)> = Vec::new();
    let mut fonts: Vec<FontFacet> = Vec::new();
    let mut fills: Vec<Option<u32>> = Vec::new();
    let mut styles: Vec<CellXf> = Vec::new();
    // `<color>` appears inside both `<font>` and `<patternFill>`, and `<xf>`
    // appears inside both `<cellStyleXfs>` and `<cellXfs>`, so every rule below
    // is guarded by which section is open. A reader that matched on local names
    // alone would take the cell-style defaults for the cell formats.
    let mut section = Section::None;
    let mut font: Option<FontFacet> = None;
    let mut pattern_solid = false;
    let mut fill_argb: Option<u32> = None;
    while let Some(event) = reader.next() {
        match event.map_err(|e| bad(part, e))? {
            Event::Start(element) => match (section, element.local_name()) {
                (_, "numFmt") => {
                    if let (Some(id), Some(code)) = (
                        element.attribute("numFmtId").and_then(|v| v.parse().ok()),
                        element.attribute("formatCode"),
                    ) {
                        custom.push((id, code.to_string()));
                    }
                }
                (Section::None, "fonts") => section = Section::Fonts,
                (Section::None, "fills") => section = Section::Fills,
                (Section::None, "cellXfs") => section = Section::CellXfs,
                (Section::Fonts, "font") => font = Some(default_font()),
                (Section::Fonts, name) => {
                    if let Some(f) = font.as_mut() {
                        apply_font_child(f, name, &element);
                    }
                }
                (Section::Fills, "patternFill") => {
                    pattern_solid = element.attribute("patternType") == Some("solid");
                    fill_argb = None;
                }
                (Section::Fills, "fgColor") => {
                    fill_argb = element.attribute("rgb").and_then(parse_argb);
                }
                (Section::CellXfs, "xf") => {
                    let id: u32 = element
                        .attribute("numFmtId")
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(0);
                    let number_format = custom
                        .iter()
                        .find(|(candidate, _)| *candidate == id)
                        .map(|(_, code)| code.clone())
                        .or_else(|| builtin_format(id).map(String::from))
                        .unwrap_or_default();
                    let font_id = index_attr(&element, "fontId");
                    let fill_id = index_attr(&element, "fillId");
                    styles.push(CellXf {
                        number_format,
                        font: font_id
                            .filter(|i| *i != 0)
                            .and_then(|i| fonts.get(i).cloned()),
                        fill: fill_id
                            .filter(|i| *i > 1)
                            .and_then(|i| fills.get(i).copied())
                            .flatten(),
                        alignment: None,
                    });
                }
                (Section::CellXfs, "alignment") => {
                    if let Some(xf) = styles.last_mut() {
                        xf.alignment = read_alignment(&element);
                    }
                }
                _ => {}
            },
            Event::End(name) => match (section, usk_xml::local(&name)) {
                (Section::Fonts, "font") => {
                    if let Some(f) = font.take() {
                        fonts.push(f);
                    }
                }
                (Section::Fills, "fill") => {
                    fills.push(if pattern_solid { fill_argb } else { None });
                    pattern_solid = false;
                    fill_argb = None;
                }
                (Section::Fonts, "fonts")
                | (Section::Fills, "fills")
                | (Section::CellXfs, "cellXfs") => section = Section::None,
                _ => {}
            },
            _ => {}
        }
    }
    Ok(styles)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Section {
    None,
    Fonts,
    Fills,
    CellXfs,
}

/// Excel's default: 11pt Calibri, black, no bits set. A `<font>` is read as a
/// delta from this, because that is how XLSX writes it — an absent `<b/>` means
/// not bold, not "unspecified".
fn default_font() -> FontFacet {
    FontFacet {
        flags: 0,
        half_points: 22,
        argb: 0xFF00_0000,
        name: String::from("Calibri"),
    }
}

fn apply_font_child(font: &mut FontFacet, name: &str, element: &usk_xml::Element) {
    // `<b/>` means bold; `<b val="0"/>` means explicitly not bold. Both occur.
    let on = element.attribute("val").map(is_true).unwrap_or(true);
    match name {
        "b" => set_flag(font, usk_oplog::FONT_BOLD, on),
        "i" => set_flag(font, usk_oplog::FONT_ITALIC, on),
        "u" => set_flag(font, usk_oplog::FONT_UNDERLINE, on),
        "strike" => set_flag(font, usk_oplog::FONT_STRIKE, on),
        "sz" => {
            if let Some(points) = element.attribute("val").and_then(|v| v.parse::<f64>().ok()) {
                // Half-points, rounded. Excel's UI offers half-point sizes and
                // the wire format is points, so this is the finest integer that
                // loses nothing. Rounded by hand rather than with `f64::round`,
                // which is std-only — the kernel is `no_std` (DP-A3).
                let half = points * 2.0;
                // NaN falls through to 0 by taking the `else` of `> 0.0`, which
                // is written positively so the partially-ordered comparison is
                // readable rather than negated.
                font.half_points = if half > 0.0 {
                    if half >= 65_534.5 {
                        65_535
                    } else {
                        (half + 0.5) as u16
                    }
                } else {
                    0
                };
            }
        }
        "color" => {
            if let Some(argb) = element.attribute("rgb").and_then(parse_argb) {
                font.argb = argb;
            }
        }
        "name" | "rFont" => {
            if let Some(value) = element.attribute("val") {
                font.name = value.to_string();
            }
        }
        _ => {}
    }
}

fn set_flag(font: &mut FontFacet, bit: u8, on: bool) {
    if on {
        font.flags |= bit;
    } else {
        font.flags &= !bit;
    }
}

fn is_true(value: &str) -> bool {
    value == "1" || value.eq_ignore_ascii_case("true")
}

fn index_attr(element: &usk_xml::Element, name: &str) -> Option<usize> {
    element.attribute(name).and_then(|v| v.parse().ok())
}

/// `"FFFF0000"` → `0xFFFF0000`. A 6-digit form (no alpha) is taken as opaque,
/// which is what every consumer does with it.
fn parse_argb(text: &str) -> Option<u32> {
    let text = text.trim();
    match text.len() {
        8 => u32::from_str_radix(text, 16).ok(),
        6 => u32::from_str_radix(text, 16).ok().map(|v| v | 0xFF00_0000),
        _ => None,
    }
}

/// An `<alignment>` that says nothing is not an alignment: `None` keeps
/// "explicitly default" and "unstyled" the same document.
fn read_alignment(element: &usk_xml::Element) -> Option<Alignment> {
    let horizontal = match element.attribute("horizontal") {
        None | Some("general") => 0,
        Some("left") => 1,
        Some("center") | Some("centre") => 2,
        Some("right") => 3,
        // `fill`, `justify`, `centerContinuous`, `distributed` are outside the
        // modelled vocabulary (TD-75); they read as general rather than as a
        // guess at which of the four they resemble.
        Some(_) => 0,
    };
    let vertical = match element.attribute("vertical") {
        None | Some("bottom") => 0,
        Some("top") => 1,
        Some("center") | Some("centre") => 2,
        Some(_) => 0,
    };
    let wrap = element.attribute("wrapText").map(is_true).unwrap_or(false);
    if horizontal == 0 && vertical == 0 && !wrap {
        return None;
    }
    Some(Alignment {
        horizontal,
        vertical,
        wrap,
    })
}

/// The built-in number formats (ECMA-376 §18.8.30). Only the ids Excel actually
/// emits are listed; an unlisted id resolves to no format, which the report
/// records rather than inventing a code for.
fn builtin_format(id: u32) -> Option<&'static str> {
    Some(match id {
        0 => return None, // "General" is the absence of a format, not a format
        1 => "0",
        2 => "0.00",
        3 => "#,##0",
        4 => "#,##0.00",
        9 => "0%",
        10 => "0.00%",
        11 => "0.00E+00",
        12 => "# ?/?",
        13 => "# ??/??",
        14 => "mm-dd-yy",
        15 => "d-mmm-yy",
        16 => "d-mmm",
        17 => "mmm-yy",
        18 => "h:mm AM/PM",
        19 => "h:mm:ss AM/PM",
        20 => "h:mm",
        21 => "h:mm:ss",
        22 => "m/d/yy h:mm",
        37 => "#,##0 ;(#,##0)",
        38 => "#,##0 ;[Red](#,##0)",
        39 => "#,##0.00;(#,##0.00)",
        40 => "#,##0.00;[Red](#,##0.00)",
        45 => "mm:ss",
        46 => "[h]:mm:ss",
        47 => "mmss.0",
        48 => "##0.0E+0",
        49 => "@",
        _ => return None,
    })
}

fn read_sheet(
    bytes: &[u8],
    part: &str,
    shared: &[String],
    formats: &[CellXf],
    fidelity: &mut Fidelity,
) -> Result<Vec<Cell>, XlsxError> {
    let mut reader = Reader::new(bytes);
    let mut cells = Vec::new();

    let mut reference = String::new();
    let mut cell_type = String::new();
    let mut style: Option<usize> = None;
    let mut in_cell = false;
    let mut in_value = false;
    let mut in_formula = false;
    let mut in_inline_text = false;
    let mut value_text = String::new();
    let mut formula_text = String::new();
    let mut inline_text = String::new();

    while let Some(event) = reader.next() {
        match event.map_err(|e| bad(part, e))? {
            Event::Start(element) => match element.local_name() {
                "c" => {
                    in_cell = true;
                    reference = element.attribute("r").unwrap_or("").to_string();
                    cell_type = element.attribute("t").unwrap_or("n").to_string();
                    style = element.attribute("s").and_then(|v| v.parse().ok());
                    value_text.clear();
                    formula_text.clear();
                    inline_text.clear();
                }
                "v" if in_cell => in_value = true,
                "f" if in_cell => in_formula = true,
                "t" if in_cell => in_inline_text = true,
                _ => {}
            },
            Event::Text(text) => {
                if in_value {
                    value_text.push_str(&text);
                } else if in_formula {
                    formula_text.push_str(&text);
                } else if in_inline_text {
                    inline_text.push_str(&text);
                }
            }
            Event::End(name) => match usk_xml::local(&name) {
                "v" => in_value = false,
                "f" => in_formula = false,
                "t" => in_inline_text = false,
                "c" if in_cell => {
                    in_cell = false;
                    if let Some(cell) = build_cell(
                        part,
                        &reference,
                        &cell_type,
                        style,
                        &value_text,
                        &formula_text,
                        &inline_text,
                        shared,
                        formats,
                        fidelity,
                    ) {
                        cells.push(cell);
                    }
                }
                _ => {}
            },
        }
    }
    Ok(cells)
}

#[allow(clippy::too_many_arguments)]
fn build_cell(
    part: &str,
    reference: &str,
    cell_type: &str,
    style: Option<usize>,
    value_text: &str,
    formula_text: &str,
    inline_text: &str,
    shared: &[String],
    formats: &[CellXf],
    fidelity: &mut Fidelity,
) -> Option<Cell> {
    // `parse_a1` is the engine's own, not a second implementation: a reader
    // that disagreed with the evaluator about what `A1` means would be a very
    // quiet bug.
    let Some(a1) = usk_formula::parse::parse_a1(reference) else {
        fidelity.losses.push(Loss {
            part: part.to_string(),
            reference: reference.to_string(),
            reason: LossReason::UnparseableReference,
        });
        return None;
    };

    let xf = match style {
        Some(index) => match formats.get(index) {
            Some(xf) => Some(xf.clone()),
            None => {
                fidelity.losses.push(Loss {
                    part: part.to_string(),
                    reference: reference.to_string(),
                    reason: LossReason::UnresolvedStyle,
                });
                None
            }
        },
        None => None,
    };
    let number_format = xf
        .as_ref()
        .map(|x| x.number_format.clone())
        .filter(|code| !code.is_empty());
    let font = xf.as_ref().and_then(|x| x.font.clone());
    let fill = xf.as_ref().and_then(|x| x.fill);
    let alignment = xf.as_ref().and_then(|x| x.alignment);

    let value = match cell_type {
        // Shared string: `<v>` is an index into the table.
        "s" => match value_text
            .parse::<usize>()
            .ok()
            .and_then(|index| shared.get(index))
        {
            Some(text) => Value::Text(text.clone()),
            None => {
                fidelity.losses.push(Loss {
                    part: part.to_string(),
                    reference: reference.to_string(),
                    reason: LossReason::SharedStringOutOfRange,
                });
                Value::Blank
            }
        },
        "inlineStr" => Value::Text(inline_text.to_string()),
        // A formula whose cached result is text.
        "str" => Value::Text(value_text.to_string()),
        "b" => Value::Bool(value_text == "1" || value_text.eq_ignore_ascii_case("true")),
        "e" => Value::Error(CellError::new(error_kind(value_text), Origin::Authored)),
        // ISO-8601 dates. Rare in practice (Excel writes serials) and cheap to
        // keep as text rather than invent a conversion the date layer has not
        // decided on yet (D-043).
        "d" => Value::Text(value_text.to_string()),
        // "n" and anything unrecognised: a number, which is XLSX's default.
        "n" | "" => {
            if value_text.is_empty() {
                Value::Blank
            } else {
                match value_text.parse::<f64>() {
                    Ok(n) => Value::Number(n),
                    Err(_) => {
                        fidelity.losses.push(Loss {
                            part: part.to_string(),
                            reference: reference.to_string(),
                            reason: LossReason::UnparseableValue,
                        });
                        Value::Text(value_text.to_string())
                    }
                }
            }
        }
        _ => {
            fidelity.losses.push(Loss {
                part: part.to_string(),
                reference: reference.to_string(),
                reason: LossReason::UnsupportedCellType,
            });
            Value::Text(value_text.to_string())
        }
    };

    let formula = if formula_text.is_empty() {
        None
    } else {
        Some(formula_text.to_string())
    };

    // A cell with nothing in it at all carries no information; XLSX emits them
    // to hold a style, and keeping them would put empty cells in every import.
    // "A style" now means any facet, not just a number format — dropping a
    // blank cell that exists only to be yellow would lose the yellow.
    if matches!(value, Value::Blank)
        && formula.is_none()
        && number_format.is_none()
        && font.is_none()
        && fill.is_none()
        && alignment.is_none()
    {
        return None;
    }

    Some(Cell {
        row: a1.row,
        col: a1.col,
        value,
        formula,
        number_format,
        font,
        fill,
        alignment,
    })
}

fn error_kind(text: &str) -> ErrorKind {
    match text {
        "#DIV/0!" => ErrorKind::Div0,
        "#VALUE!" => ErrorKind::Value,
        "#REF!" => ErrorKind::Ref,
        "#NAME?" => ErrorKind::Name,
        "#NUM!" => ErrorKind::Num,
        "#NULL!" => ErrorKind::Value,
        "#SPILL!" => ErrorKind::Spill,
        // `#N/A` and anything unrecognised. `#N/A` is the common case and the
        // fallback is the least-wrong answer for a spelling we do not know.
        _ => ErrorKind::Na,
    }
}

fn bad(part: &str, err: usk_xml::XmlError) -> XlsxError {
    XlsxError::BadPart {
        part: part.to_string(),
        detail: format!("{err:?}"),
    }
}
