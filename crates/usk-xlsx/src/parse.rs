//! Reading the parts: workbook, relationships, shared strings, styles, sheets.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use usk_types::{CellError, ErrorKind, Origin, Value};
use usk_xml::{Event, Reader};
use usk_zip::Archive;

use crate::{
    is_active_content, is_known_unmodelled, Cell, Fidelity, Loss, LossReason, Sheet, Workbook,
    XlsxError,
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

/// `styles.xml` → the number-format code for each style index.
///
/// Two levels of indirection, both of which XLSX requires: a cell names a
/// `cellXfs` index, that entry names a `numFmtId`, and the id is either a
/// built-in or defined in `numFmts`.
fn read_styles(archive: &Archive, fidelity: &mut Fidelity) -> Result<Vec<String>, XlsxError> {
    let part = "xl/styles.xml";
    let Some(bytes) = archive.read_named(part) else {
        return Ok(Vec::new());
    };
    let bytes = bytes.map_err(XlsxError::Zip)?;
    fidelity.parts_read.push(part.to_string());

    let mut reader = Reader::new(&bytes);
    let mut custom: Vec<(u32, String)> = Vec::new();
    let mut styles: Vec<String> = Vec::new();
    let mut in_cell_xfs = false;
    while let Some(event) = reader.next() {
        match event.map_err(|e| bad(part, e))? {
            Event::Start(element) => match element.local_name() {
                "numFmt" => {
                    if let (Some(id), Some(code)) = (
                        element.attribute("numFmtId").and_then(|v| v.parse().ok()),
                        element.attribute("formatCode"),
                    ) {
                        custom.push((id, code.to_string()));
                    }
                }
                "cellXfs" => in_cell_xfs = true,
                "xf" if in_cell_xfs => {
                    let id: u32 = element
                        .attribute("numFmtId")
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(0);
                    let code = custom
                        .iter()
                        .find(|(candidate, _)| *candidate == id)
                        .map(|(_, code)| code.clone())
                        .or_else(|| builtin_format(id).map(String::from))
                        .unwrap_or_default();
                    styles.push(code);
                }
                _ => {}
            },
            Event::End(name) if usk_xml::local(&name) == "cellXfs" => in_cell_xfs = false,
            _ => {}
        }
    }
    Ok(styles)
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
    formats: &[String],
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
    formats: &[String],
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

    let number_format = match style {
        Some(index) => match formats.get(index) {
            Some(code) if !code.is_empty() => Some(code.clone()),
            Some(_) => None,
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
    if matches!(value, Value::Blank) && formula.is_none() && number_format.is_none() {
        return None;
    }

    Some(Cell {
        row: a1.row,
        col: a1.col,
        value,
        formula,
        number_format,
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
