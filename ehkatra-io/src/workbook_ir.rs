//! The XLSX half of the IR (docs/24: *IR-only output revalidated against
//! schema by the host*).
//!
//! Same contract as the CSV half in [`crate::ir`]: a closed vocabulary with no
//! verbs, and every bound the child was supposed to enforce re-checked here.
//! The child has just parsed a ZIP full of compressed XML from an untrusted
//! source; if anything got past it, this is the boundary that has to notice.

use usk_json::{number, string, Json};
use usk_types::{CellError, ErrorKind, Origin, Value};
use usk_xlsx::{Cell, Fidelity, Loss, LossReason, Sheet, Workbook};

use crate::ir::IrError;

/// Cells accepted from one container. Excel's own limit is 2^34 per sheet; this
/// is a memory bound on a host reading a child's output, not a format limit.
pub const MAX_CELLS: usize = 4_000_000;
/// Sheets accepted from one container.
pub const MAX_SHEETS: usize = 4096;

pub fn encode(workbook: &Workbook) -> Json {
    Json::Object(vec![
        (
            String::from("sheets"),
            Json::Array(workbook.sheets.iter().map(encode_sheet).collect()),
        ),
        (
            String::from("fidelity"),
            encode_fidelity(&workbook.fidelity),
        ),
    ])
}

fn encode_sheet(sheet: &Sheet) -> Json {
    Json::Object(vec![
        (String::from("name"), string(&sheet.name)),
        (String::from("part"), string(&sheet.part)),
        (
            String::from("cells"),
            Json::Array(sheet.cells.iter().map(encode_cell).collect()),
        ),
    ])
}

fn encode_cell(cell: &Cell) -> Json {
    let mut fields = vec![
        (String::from("r"), number(cell.row as f64)),
        (String::from("c"), number(cell.col as f64)),
        (String::from("v"), encode_value(&cell.value)),
    ];
    if let Some(formula) = &cell.formula {
        fields.push((String::from("f"), string(formula)));
    }
    if let Some(format) = &cell.number_format {
        fields.push((String::from("nf"), string(format)));
    }
    Json::Object(fields)
}

/// A tagged value. The tag is explicit rather than inferred from the JSON type
/// because `Blank` and an empty string are different cells, and a reader that
/// guesses gets that wrong in the direction users notice.
fn encode_value(value: &Value) -> Json {
    let (tag, payload) = match value {
        Value::Blank => ("z", Json::Null),
        Value::Bool(b) => ("b", Json::Bool(*b)),
        Value::Number(n) => ("n", number(*n)),
        Value::Decimal(d) => ("n", number(d.to_f64())),
        Value::Text(s) => ("s", string(s)),
        Value::Error(e) => ("e", string(e.kind.as_str())),
    };
    Json::Object(vec![
        (String::from("t"), string(tag)),
        (String::from("d"), payload),
    ])
}

fn encode_fidelity(fidelity: &Fidelity) -> Json {
    let names = |items: &[String]| Json::Array(items.iter().map(string).collect());
    Json::Object(vec![
        (
            String::from("parts_total"),
            number(fidelity.parts_total as f64),
        ),
        (String::from("parts_read"), names(&fidelity.parts_read)),
        (
            String::from("parts_ignored"),
            names(&fidelity.parts_ignored),
        ),
        (
            String::from("parts_structural"),
            names(&fidelity.parts_structural),
        ),
        (String::from("quarantined"), names(&fidelity.quarantined)),
        (
            String::from("cells_read"),
            number(fidelity.cells_read as f64),
        ),
        (
            String::from("formulas_read"),
            number(fidelity.formulas_read as f64),
        ),
        (
            String::from("number_formats_resolved"),
            number(fidelity.number_formats_resolved as f64),
        ),
        (
            String::from("losses"),
            Json::Array(fidelity.losses.iter().map(encode_loss).collect()),
        ),
    ])
}

fn encode_loss(loss: &Loss) -> Json {
    Json::Object(vec![
        (String::from("part"), string(&loss.part)),
        (String::from("reference"), string(&loss.reference)),
        (String::from("reason"), string(loss_name(loss.reason))),
    ])
}

fn loss_name(reason: LossReason) -> &'static str {
    match reason {
        LossReason::UnsupportedCellType => "UnsupportedCellType",
        LossReason::UnresolvedStyle => "UnresolvedStyle",
        LossReason::SharedStringOutOfRange => "SharedStringOutOfRange",
        LossReason::UnparseableReference => "UnparseableReference",
        LossReason::UnparseableValue => "UnparseableValue",
    }
}

// ---------------------------------------------------------------- decoding

pub fn decode(json: &Json) -> Result<Workbook, IrError> {
    let sheets_json = json
        .get("sheets")
        .and_then(Json::as_array)
        .ok_or(IrError::MissingField("workbook.sheets"))?;
    if sheets_json.len() > MAX_SHEETS {
        return Err(IrError::BoundViolated("MAX_SHEETS"));
    }

    let mut total_cells = 0usize;
    let mut sheets = Vec::with_capacity(sheets_json.len());
    for sheet in sheets_json {
        let cells_json = sheet
            .get("cells")
            .and_then(Json::as_array)
            .ok_or(IrError::MissingField("workbook.sheets[].cells"))?;
        total_cells += cells_json.len();
        if total_cells > MAX_CELLS {
            return Err(IrError::BoundViolated("MAX_CELLS"));
        }
        let mut cells = Vec::with_capacity(cells_json.len());
        for cell in cells_json {
            cells.push(decode_cell(cell)?);
        }
        sheets.push(Sheet {
            name: text(sheet.get("name")),
            part: text(sheet.get("part")),
            cells,
        });
    }

    let fidelity = decode_fidelity(
        json.get("fidelity")
            .ok_or(IrError::MissingField("workbook.fidelity"))?,
    )?;
    // The child's own accounting has to add up. A report that claims more cells
    // than it sent is either broken or lying, and both mean its output goes in
    // the bin rather than into a workbook.
    if fidelity.cells_read != total_cells {
        return Err(IrError::BoundViolated("cells_read disagrees with cells"));
    }
    if fidelity.parts_read.len()
        + fidelity.parts_ignored.len()
        + fidelity.parts_structural.len()
        + fidelity.quarantined.len()
        > fidelity.parts_total
    {
        return Err(IrError::BoundViolated("part counts exceed parts_total"));
    }

    Ok(Workbook { sheets, fidelity })
}

fn decode_cell(json: &Json) -> Result<Cell, IrError> {
    Ok(Cell {
        row: index(json.get("r"))?,
        col: index(json.get("c"))?,
        value: decode_value(json.get("v").ok_or(IrError::MissingField("cell.v"))?),
        formula: json.get("f").and_then(Json::as_str).map(String::from),
        number_format: json.get("nf").and_then(Json::as_str).map(String::from),
    })
}

fn decode_value(json: &Json) -> Value {
    let payload = json.get("d");
    match json.get("t").and_then(Json::as_str) {
        Some("b") => Value::Bool(payload.and_then(Json::as_bool).unwrap_or(false)),
        Some("n") => Value::Number(payload.and_then(Json::as_f64).unwrap_or(0.0)),
        Some("s") => Value::Text(text(payload)),
        Some("e") => Value::Error(CellError::new(error_kind(&text(payload)), Origin::Authored)),
        // `z` and anything unrecognised. Blank is the value that asserts
        // nothing, which is the right answer for a tag we do not know.
        _ => Value::Blank,
    }
}

fn error_kind(name: &str) -> ErrorKind {
    match name {
        "#DIV/0!" => ErrorKind::Div0,
        "#VALUE!" => ErrorKind::Value,
        "#REF!" => ErrorKind::Ref,
        "#NAME?" => ErrorKind::Name,
        "#NUM!" => ErrorKind::Num,
        "#CIRC!" => ErrorKind::Circ,
        "#SPILL!" => ErrorKind::Spill,
        _ => ErrorKind::Na,
    }
}

fn decode_fidelity(json: &Json) -> Result<Fidelity, IrError> {
    let names = |key: &str| -> Vec<String> {
        json.get(key)
            .and_then(Json::as_array)
            .unwrap_or(&[])
            .iter()
            .filter_map(Json::as_str)
            .map(String::from)
            .collect()
    };
    let losses = json
        .get("losses")
        .and_then(Json::as_array)
        .unwrap_or(&[])
        .iter()
        .map(|loss| Loss {
            part: text(loss.get("part")),
            reference: text(loss.get("reference")),
            reason: loss_of(loss.get("reason").and_then(Json::as_str).unwrap_or("")),
        })
        .collect();

    Ok(Fidelity {
        parts_total: count(json.get("parts_total")),
        parts_read: names("parts_read"),
        parts_ignored: names("parts_ignored"),
        parts_structural: names("parts_structural"),
        quarantined: names("quarantined"),
        cells_read: count(json.get("cells_read")),
        formulas_read: count(json.get("formulas_read")),
        number_formats_resolved: count(json.get("number_formats_resolved")),
        losses,
    })
}

fn loss_of(name: &str) -> LossReason {
    match name {
        "UnresolvedStyle" => LossReason::UnresolvedStyle,
        "SharedStringOutOfRange" => LossReason::SharedStringOutOfRange,
        "UnparseableReference" => LossReason::UnparseableReference,
        "UnparseableValue" => LossReason::UnparseableValue,
        _ => LossReason::UnsupportedCellType,
    }
}

fn index(json: Option<&Json>) -> Result<u32, IrError> {
    let n = json
        .and_then(Json::as_f64)
        .ok_or(IrError::MissingField("cell.r/c"))?;
    if !n.is_finite() || n < 0.0 || n > u32::MAX as f64 {
        return Err(IrError::BoundViolated("cell index out of range"));
    }
    Ok(n as u32)
}

fn count(json: Option<&Json>) -> usize {
    json.and_then(Json::as_f64)
        .filter(|n| n.is_finite() && *n >= 0.0)
        .map(|n| n as usize)
        .unwrap_or(0)
}

fn text(json: Option<&Json>) -> String {
    json.and_then(Json::as_str)
        .map(String::from)
        .unwrap_or_default()
}
