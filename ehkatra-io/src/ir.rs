//! The intermediate representation the sandboxed parser emits, and the host's
//! revalidation of it (docs/24: *"**IR-only output revalidated against schema**
//! by the host"*).
//!
//! # Why the host re-checks bounds the child already checked
//! Because the child is the untrusted party. It has just processed a hostile
//! file in its own address space; if it was compromised, its output is the
//! attacker's output. Re-running the caps on the way in is the difference
//! between a sandbox and a speed bump, and it costs a linear pass.
//!
//! The IR is deliberately narrow — records, a report, or a named error. There
//! is no field in it that says "do this", which is the other half of the rule:
//! a compromised parser can lie about a file's *contents* but cannot ask the
//! host to take an action, because the vocabulary has none.

use usk_csv::infer::{ColumnReport, Decision, Loss, LossSample, Report};
use usk_csv::inject::{Finding, Risk};
use usk_csv::{limits, CsvError, Dialect, Record};
use usk_json::{number, string, Json};

pub const SCHEMA: &str = "ehkatra.import.ir/1";

/// What the parser produced.
///
/// `PartialEq` but not `Eq`: a workbook holds `f64` cell values, and pretending
/// those have a total equality would be a lie about floating point rather than
/// a convenience.
#[derive(Clone, PartialEq, Debug)]
pub enum Ir {
    Parsed {
        dialect: Dialect,
        records: Vec<Record>,
        report: Report,
    },
    /// An XLSX workbook and its fidelity report (BOOTSTRAP row 12).
    Workbook(usk_xlsx::Workbook),
    Failed(CsvError),
    /// A container this build could not read, as a named reason. Distinct from
    /// [`Ir::Failed`], which is CSV's vocabulary — an XLSX defect is not a CSV
    /// defect and flattening the two would lose the part name.
    WorkbookFailed(String),
}

/// Why the host would not accept a child's output.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum IrError {
    NotJson,
    WrongSchema,
    MissingField(&'static str),
    /// The child's output broke a bound the child was supposed to enforce.
    /// This is the interesting one: it means the parser is lying, which means
    /// it is compromised or broken, and either way its output is discarded.
    BoundViolated(&'static str),
}

// ------------------------------------------------------------------ encoding

pub fn encode(ir: &Ir) -> String {
    let body = match ir {
        Ir::Parsed {
            dialect,
            records,
            report,
        } => Json::Object(vec![
            (String::from("schema"), string(SCHEMA)),
            (String::from("kind"), string("csv")),
            (
                String::from("dialect"),
                Json::Object(vec![
                    (String::from("delimiter"), number(dialect.delimiter as f64)),
                    (String::from("quote"), number(dialect.quote as f64)),
                ]),
            ),
            (
                String::from("records"),
                Json::Array(records.iter().map(encode_record).collect()),
            ),
            (String::from("report"), encode_report(report)),
        ]),
        Ir::Workbook(workbook) => {
            let Json::Object(mut fields) = crate::workbook_ir::encode(workbook) else {
                // `encode` is a literal object; this arm is unreachable and is
                // written as a fallback rather than an unwrap (DP-C1).
                return Json::Object(vec![
                    (String::from("schema"), string(SCHEMA)),
                    (String::from("kind"), string("xlsx")),
                    (String::from("error"), string("encode produced no object")),
                ])
                .to_json_string();
            };
            let mut out = vec![
                (String::from("schema"), string(SCHEMA)),
                (String::from("kind"), string("xlsx")),
            ];
            out.append(&mut fields);
            Json::Object(out)
        }
        Ir::WorkbookFailed(detail) => Json::Object(vec![
            (String::from("schema"), string(SCHEMA)),
            (String::from("kind"), string("xlsx")),
            (String::from("error"), string(detail)),
        ]),
        Ir::Failed(err) => Json::Object(vec![
            (String::from("schema"), string(SCHEMA)),
            (String::from("kind"), string("csv")),
            (String::from("error"), encode_error(err)),
        ]),
    };
    body.to_json_string()
}

fn encode_record(record: &Record) -> Json {
    Json::Object(vec![
        (String::from("line"), number(record.line as f64)),
        (
            String::from("fields"),
            Json::Array(record.fields.iter().map(string).collect()),
        ),
    ])
}

fn encode_error(err: &CsvError) -> Json {
    let (kind, line, extra) = match err {
        CsvError::FieldTooLong { line, bytes } => ("FieldTooLong", *line, Some(*bytes)),
        CsvError::TooManyFields { line } => ("TooManyFields", *line, None),
        CsvError::UnterminatedQuote { line } => ("UnterminatedQuote", *line, None),
        CsvError::NotUtf8 { line } => ("NotUtf8", *line, None),
    };
    let mut fields = vec![
        (String::from("kind"), string(kind)),
        (String::from("line"), number(line as f64)),
    ];
    if let Some(bytes) = extra {
        fields.push((String::from("bytes"), number(bytes as f64)));
    }
    Json::Object(fields)
}

fn encode_report(report: &Report) -> Json {
    Json::Object(vec![
        (
            String::from("columns"),
            Json::Array(report.columns.iter().map(encode_column).collect()),
        ),
        (
            String::from("rows_sampled"),
            number(report.rows_sampled as f64),
        ),
        (String::from("rows_total"), number(report.rows_total as f64)),
        (
            String::from("injections"),
            Json::Array(report.injections.iter().map(encode_finding).collect()),
        ),
        (
            String::from("ragged_rows"),
            Json::Array(
                report
                    .ragged_rows
                    .iter()
                    .map(|l| number(*l as f64))
                    .collect(),
            ),
        ),
    ])
}

fn encode_column(column: &ColumnReport) -> Json {
    Json::Object(vec![
        (String::from("index"), number(column.index as f64)),
        (String::from("name"), string(&column.name)),
        (String::from("blank"), number(column.blank as f64)),
        (String::from("numeric"), number(column.numeric as f64)),
        (String::from("boolean"), number(column.boolean as f64)),
        (String::from("textual"), number(column.textual as f64)),
        (String::from("loss_count"), number(column.loss_count as f64)),
        (
            String::from("losses"),
            Json::Array(column.losses.iter().map(encode_loss).collect()),
        ),
        (
            String::from("suggested"),
            string(decision_name(column.suggested)),
        ),
    ])
}

fn encode_loss(sample: &LossSample) -> Json {
    Json::Object(vec![
        (String::from("line"), number(sample.line as f64)),
        (String::from("original"), string(&sample.original)),
        (String::from("as_number"), string(&sample.as_number)),
        (String::from("loss"), string(loss_name(sample.loss))),
    ])
}

fn encode_finding(finding: &Finding) -> Json {
    let (risk, lead) = match finding.risk {
        Risk::FormulaLead(c) => ("FormulaLead", c),
        Risk::ControlLead(c) => ("ControlLead", c),
    };
    Json::Object(vec![
        (String::from("line"), number(finding.line as f64)),
        (String::from("column"), number(finding.column as f64)),
        (String::from("risk"), string(risk)),
        (String::from("lead"), number(lead as u32 as f64)),
        (String::from("sample"), string(&finding.sample)),
    ])
}

fn decision_name(decision: Decision) -> &'static str {
    match decision {
        Decision::Number => "Number",
        Decision::Text => "Text",
        Decision::Boolean => "Boolean",
        Decision::PerCell => "PerCell",
    }
}

fn decision_of(name: &str) -> Decision {
    match name {
        "Number" => Decision::Number,
        "Boolean" => Decision::Boolean,
        "PerCell" => Decision::PerCell,
        // Anything unrecognised falls back to the decision that cannot lose
        // data. A compromised child must not be able to pick `Number` for a
        // column by sending a name we do not know.
        _ => Decision::Text,
    }
}

fn loss_name(loss: Loss) -> &'static str {
    match loss {
        Loss::LeadingZeros => "LeadingZeros",
        Loss::ScientificNotation => "ScientificNotation",
        Loss::TrailingZeros => "TrailingZeros",
        Loss::PrecisionBeyond15Digits => "PrecisionBeyond15Digits",
        Loss::Reformatted => "Reformatted",
    }
}

fn loss_of(name: &str) -> Loss {
    match name {
        "LeadingZeros" => Loss::LeadingZeros,
        "ScientificNotation" => Loss::ScientificNotation,
        "TrailingZeros" => Loss::TrailingZeros,
        "PrecisionBeyond15Digits" => Loss::PrecisionBeyond15Digits,
        _ => Loss::Reformatted,
    }
}

// ---------------------------------------------------------------- decoding

/// Parses and **revalidates** a child's output.
///
/// Every bound `usk_csv` enforces is re-checked here against the decoded
/// records, because the child that enforced them has just been exposed to the
/// file and is no longer trusted to have survived it.
pub fn decode(bytes: &[u8]) -> Result<Ir, IrError> {
    let doc = usk_json::parse(bytes).map_err(|_| IrError::NotJson)?;
    if doc.get("schema").and_then(Json::as_str) != Some(SCHEMA) {
        return Err(IrError::WrongSchema);
    }

    let kind = doc.get("kind").and_then(Json::as_str).unwrap_or("csv");
    if kind == "xlsx" {
        if let Some(error) = doc.get("error").and_then(Json::as_str) {
            return Ok(Ir::WorkbookFailed(String::from(error)));
        }
        return crate::workbook_ir::decode(&doc).map(Ir::Workbook);
    }

    if let Some(error) = doc.get("error") {
        let kind = error
            .get("kind")
            .and_then(Json::as_str)
            .ok_or(IrError::MissingField("error.kind"))?;
        let line = usize_of(error.get("line")).ok_or(IrError::MissingField("error.line"))?;
        return Ok(Ir::Failed(match kind {
            "FieldTooLong" => CsvError::FieldTooLong {
                line,
                bytes: usize_of(error.get("bytes")).unwrap_or(0),
            },
            "TooManyFields" => CsvError::TooManyFields { line },
            "NotUtf8" => CsvError::NotUtf8 { line },
            // An unknown error name is reported as the most conservative one:
            // "we could not read this file" is always true when the child
            // failed, and inventing a specific cause would be a lie.
            _ => CsvError::UnterminatedQuote { line },
        }));
    }

    let dialect = doc
        .get("dialect")
        .ok_or(IrError::MissingField("dialect"))
        .map(|d| Dialect {
            delimiter: usize_of(d.get("delimiter")).unwrap_or(b',' as usize) as u8,
            quote: usize_of(d.get("quote")).unwrap_or(b'"' as usize) as u8,
        })?;

    let raw_records = doc
        .get("records")
        .and_then(Json::as_array)
        .ok_or(IrError::MissingField("records"))?;
    let mut records = Vec::with_capacity(raw_records.len());
    for raw in raw_records {
        let fields_json = raw
            .get("fields")
            .and_then(Json::as_array)
            .ok_or(IrError::MissingField("records[].fields"))?;
        if fields_json.len() > limits::MAX_FIELDS {
            return Err(IrError::BoundViolated("MAX_FIELDS"));
        }
        let mut fields = Vec::with_capacity(fields_json.len());
        for field in fields_json {
            let text = field
                .as_str()
                .ok_or(IrError::MissingField("records[].fields[]"))?;
            if text.len() > limits::MAX_FIELD_BYTES {
                return Err(IrError::BoundViolated("MAX_FIELD_BYTES"));
            }
            fields.push(String::from(text));
        }
        records.push(Record {
            fields,
            line: usize_of(raw.get("line")).unwrap_or(0),
        });
    }

    let report = decode_report(doc.get("report").ok_or(IrError::MissingField("report"))?)?;
    if report.rows_sampled > report.rows_total {
        return Err(IrError::BoundViolated("rows_sampled > rows_total"));
    }
    if report.columns.len() > limits::MAX_FIELDS {
        return Err(IrError::BoundViolated("MAX_FIELDS"));
    }

    Ok(Ir::Parsed {
        dialect,
        records,
        report,
    })
}

fn decode_report(json: &Json) -> Result<Report, IrError> {
    let columns_json = json
        .get("columns")
        .and_then(Json::as_array)
        .ok_or(IrError::MissingField("report.columns"))?;
    let mut columns = Vec::with_capacity(columns_json.len());
    for column in columns_json {
        let losses_json = column.get("losses").and_then(Json::as_array).unwrap_or(&[]);
        let losses = losses_json
            .iter()
            .map(|loss| LossSample {
                line: usize_of(loss.get("line")).unwrap_or(0),
                original: text_of(loss.get("original")),
                as_number: text_of(loss.get("as_number")),
                loss: loss_of(loss.get("loss").and_then(Json::as_str).unwrap_or("")),
            })
            .collect();
        columns.push(ColumnReport {
            index: usize_of(column.get("index")).unwrap_or(0),
            name: text_of(column.get("name")),
            blank: usize_of(column.get("blank")).unwrap_or(0),
            numeric: usize_of(column.get("numeric")).unwrap_or(0),
            boolean: usize_of(column.get("boolean")).unwrap_or(0),
            textual: usize_of(column.get("textual")).unwrap_or(0),
            losses,
            loss_count: usize_of(column.get("loss_count")).unwrap_or(0),
            suggested: decision_of(column.get("suggested").and_then(Json::as_str).unwrap_or("")),
        });
    }

    let injections = json
        .get("injections")
        .and_then(Json::as_array)
        .unwrap_or(&[])
        .iter()
        .map(|finding| {
            let lead =
                char::from_u32(usize_of(finding.get("lead")).unwrap_or(0) as u32).unwrap_or('?');
            let risk = match finding.get("risk").and_then(Json::as_str) {
                Some("ControlLead") => Risk::ControlLead(lead),
                _ => Risk::FormulaLead(lead),
            };
            Finding::new(
                usize_of(finding.get("line")).unwrap_or(0),
                usize_of(finding.get("column")).unwrap_or(0),
                risk,
                &text_of(finding.get("sample")),
            )
        })
        .collect();

    let ragged_rows = json
        .get("ragged_rows")
        .and_then(Json::as_array)
        .unwrap_or(&[])
        .iter()
        .map(|l| usize_of(Some(l)).unwrap_or(0))
        .collect();

    Ok(Report {
        columns,
        rows_sampled: usize_of(json.get("rows_sampled")).unwrap_or(0),
        rows_total: usize_of(json.get("rows_total")).unwrap_or(0),
        injections,
        ragged_rows,
    })
}

fn usize_of(json: Option<&Json>) -> Option<usize> {
    let n = json?.as_f64()?;
    if !n.is_finite() || n < 0.0 {
        return None;
    }
    Some(n as usize)
}

fn text_of(json: Option<&Json>) -> String {
    json.and_then(Json::as_str)
        .map(String::from)
        .unwrap_or_default()
}
