//! Writing the parts: XLSX **write** (docs/24 format matrix, session 29).
//!
//! The projection in reverse: what [`crate::read`] models — values, formulas
//! with their cached results, number formats at both indirection levels —
//! is exactly what this writes, and nothing else is claimed. The output is a
//! minimum honest OOXML package: `[Content_Types].xml`, the root relationship,
//! `xl/workbook.xml` + its rels, one worksheet part per sheet,
//! `xl/sharedStrings.xml` when any literal text exists, and an
//! `xl/styles.xml` that models **number formats only** (the font/fill/border
//! entries in it are the empty skeleton Excel requires a styles part to have,
//! not a claim about formatting).
//!
//! # Say what was lost
//! The reader's fidelity report names what a file gave up on the way in; the
//! writer holds itself to the same rule on the way out. [`WriteReport`] names
//! every source part not re-emitted (charts, themes — and active content,
//! which docs/24 forbids re-emitting by default) and every cell that could not
//! cross losslessly: a non-finite number (XLSX has no spelling; Excel stores
//! `#NUM!`, so we do too), a `Decimal` (written as its exact decimal text,
//! which any XLSX reader — ours included — will take up as a binary double),
//! and `#CIRC!` (an engine-internal error kind with no XLSX vocabulary entry;
//! written as `#N/A`). Nothing is dropped silently.
//!
//! # Determinism (DP-A2)
//! The same `Workbook` produces the same bytes: cells are emitted in
//! `(row, col)` order, shared strings and number formats in first-use order,
//! and the container (usk-zip's stored writer) stamps a fixed timestamp.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::fmt::Write as _;

use usk_formula::parse::A1;
use usk_formula::translate::render as render_a1;
use usk_types::{ErrorKind, Value};
use usk_zip::ZipError;

use crate::{Alignment, Cell, FontFacet, Sheet, Workbook};

/// The container bytes, plus the honest account of producing them.
#[derive(Clone, PartialEq, Debug)]
pub struct Written {
    pub bytes: Vec<u8>,
    pub report: WriteReport,
}

/// What the writer emitted and what it could not carry — the write-side
/// sibling of [`crate::Fidelity`], and like it a deliverable rather than a
/// diagnostic.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct WriteReport {
    pub parts_written: Vec<String>,
    /// Source parts (from the workbook's read fidelity) this writer does not
    /// re-emit — charts, drawings, themes. Named individually, same rule as
    /// the reader's `parts_ignored`.
    pub parts_dropped: Vec<String>,
    /// docs/24 active content: *never re-emitted by default*. Listed so the
    /// policy is visible in the report, not just in the policy document.
    pub quarantined_dropped: Vec<String>,
    /// Source parts the read fidelity could not name (present in the container
    /// but neither read, ignored, quarantined nor structural). They cannot be
    /// re-emitted because their bytes were never kept; the count keeps the
    /// report's arithmetic honest.
    pub parts_unaccounted: usize,
    pub cells_written: usize,
    pub formulas_written: usize,
    pub number_formats_written: usize,
    /// Cells carrying any facet — the ADR-041 surface, counted beside the
    /// number-format figure rather than replacing it, so session 29's number
    /// stays comparable with itself.
    pub styles_written: usize,
    pub fonts_written: usize,
    pub fills_written: usize,
    pub alignments_written: usize,
    pub shared_strings: usize,
    /// Cell-level state that did not survive, each with a named reason.
    pub losses: Vec<WriteLoss>,
}

impl WriteReport {
    pub fn is_lossless(&self) -> bool {
        self.losses.is_empty()
            && self.parts_dropped.is_empty()
            && self.quarantined_dropped.is_empty()
            && self.parts_unaccounted == 0
    }
}

/// A specific thing that did not survive the write.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct WriteLoss {
    pub sheet: String,
    pub reference: String,
    pub reason: WriteLossReason,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum WriteLossReason {
    /// NaN or ±infinity. XLSX has no representation; Excel itself stores
    /// `#NUM!` for an overflowed result, so the cell is written as that error.
    NonFiniteNumber,
    /// A `Value::Decimal`, written as its exact decimal text. The digits are
    /// all in the file, but XLSX numbers are doubles to every reader that will
    /// open it — including ours — so the *type* does not round-trip.
    DecimalWrittenAsNumber,
    /// An error kind XLSX cannot carry in a plain `t="e"` cell. `#CIRC!` is
    /// engine-internal and has no OOXML spelling at all; `#SPILL!` has one but
    /// Excel only accepts it alongside the rich-value metadata parts this
    /// writer does not claim — **verified against Excel via COM (session 29):
    /// a container holding a bare `#SPILL!` error cell, literal or
    /// formula-cached, is refused outright.** Both are written as `#N/A`,
    /// which is what our own reader degrades unknown spellings to anyway.
    ErrorOutsideXlsxVocabulary,
}

/// Why a workbook could not be written at all.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum WriteError {
    /// XLSX requires at least one sheet; inventing one would put a sheet in
    /// the file that is not in the model.
    NoSheets,
    Zip(ZipError),
}

impl From<ZipError> for WriteError {
    fn from(err: ZipError) -> Self {
        WriteError::Zip(err)
    }
}

/// Writes a workbook to XLSX container bytes.
///
/// The inverse of [`crate::read`] over the modelled surface, and
/// `usk-xlsx/tests/roundtrip.rs` holds it to that: read → write → re-read must
/// reproduce every sheet name, address, value, formula text and number format,
/// with every exception named in the [`WriteReport`].
pub fn write(book: &Workbook) -> Result<Written, WriteError> {
    if book.sheets.is_empty() {
        return Err(WriteError::NoSheets);
    }

    let mut report = WriteReport {
        parts_dropped: book.fidelity.parts_ignored.clone(),
        quarantined_dropped: book.fidelity.quarantined.clone(),
        parts_unaccounted: book.fidelity.parts_total.saturating_sub(
            book.fidelity.parts_read.len()
                + book.fidelity.parts_ignored.len()
                + book.fidelity.quarantined.len()
                + book.fidelity.parts_structural.len(),
        ),
        ..WriteReport::default()
    };

    // Cells are emitted sorted, so everything derived from a walk over them —
    // shared-string order, format order, the XML itself — is a pure function
    // of the workbook (DP-A2).
    let sorted: Vec<Vec<&Cell>> = book
        .sheets
        .iter()
        .map(|sheet| {
            let mut cells: Vec<&Cell> = sheet.cells.iter().collect();
            cells.sort_by_key(|c| (c.row, c.col));
            cells
        })
        .collect();

    // The style tables, first-use order over cells already sorted by
    // `(row, col)` — so every index below is a pure function of the workbook
    // (DP-A2), exactly as the shared-string table is.
    //
    // This is `cellXfs` used as the flyweight table it is (ADR-041): a cell
    // names one `xf`, an `xf` names one entry in each of the number-format,
    // font and fill tables, and identical formatting anywhere in the workbook
    // shares all four.
    let styles = StyleTables::collect(&sorted);

    // The shared-string table, first-use order. Literal text goes through the
    // table (D-122: it is what Excel itself writes, it deduplicates, and it is
    // the reader's primary text path); a formula's cached text result is
    // `t="str"` inline, which is the only place XLSX allows it.
    let mut shared: Vec<String> = Vec::new();
    for cells in &sorted {
        for cell in cells {
            if cell.formula.is_none() {
                if let Value::Text(text) = &cell.value {
                    if !shared.iter().any(|s| s == text) {
                        shared.push(text.clone());
                    }
                }
            }
        }
    }

    let mut parts: Vec<(String, Vec<u8>)> = alloc::vec![
        (
            String::from("[Content_Types].xml"),
            content_types(book.sheets.len(), !shared.is_empty()).into_bytes(),
        ),
        (String::from("_rels/.rels"), root_rels().into_bytes()),
        (
            String::from("xl/workbook.xml"),
            workbook_xml(&book.sheets).into_bytes(),
        ),
        (
            String::from("xl/_rels/workbook.xml.rels"),
            workbook_rels(book.sheets.len(), !shared.is_empty()).into_bytes(),
        ),
        (
            String::from("xl/styles.xml"),
            styles_xml(&styles).into_bytes(),
        ),
    ];
    if !shared.is_empty() {
        report.shared_strings = shared.len();
        parts.push((
            String::from("xl/sharedStrings.xml"),
            shared_strings_xml(&shared, &sorted).into_bytes(),
        ));
    }
    for (index, (sheet, cells)) in book.sheets.iter().zip(&sorted).enumerate() {
        parts.push((
            format!("xl/worksheets/sheet{}.xml", index + 1),
            sheet_xml(sheet, cells, &styles, &shared, &mut report).into_bytes(),
        ));
    }

    report.parts_written = parts.iter().map(|(name, _)| name.clone()).collect();

    let borrowed: Vec<(&str, &[u8])> = parts
        .iter()
        .map(|(name, bytes)| (name.as_str(), bytes.as_slice()))
        .collect();
    let bytes = usk_zip::write::build_stored(&borrowed)?;
    Ok(Written { bytes, report })
}

const XML_DECL: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\r\n";
const NS_MAIN: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
const NS_PKG_REL: &str = "http://schemas.openxmlformats.org/package/2006/relationships";
const NS_DOC_REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

fn content_types(sheet_count: usize, has_shared: bool) -> String {
    let mut out = String::from(XML_DECL);
    out.push_str("<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">");
    out.push_str("<Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>");
    out.push_str("<Default Extension=\"xml\" ContentType=\"application/xml\"/>");
    out.push_str("<Override PartName=\"/xl/workbook.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml\"/>");
    for index in 1..=sheet_count {
        let _ = write!(
            out,
            "<Override PartName=\"/xl/worksheets/sheet{index}.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml\"/>"
        );
    }
    out.push_str("<Override PartName=\"/xl/styles.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml\"/>");
    if has_shared {
        out.push_str("<Override PartName=\"/xl/sharedStrings.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.sharedStrings+xml\"/>");
    }
    out.push_str("</Types>");
    out
}

fn root_rels() -> String {
    let mut out = String::from(XML_DECL);
    let _ = write!(
        out,
        "<Relationships xmlns=\"{NS_PKG_REL}\"><Relationship Id=\"rId1\" Type=\"{NS_DOC_REL}/officeDocument\" Target=\"xl/workbook.xml\"/></Relationships>"
    );
    out
}

fn workbook_xml(sheets: &[Sheet]) -> String {
    let mut out = String::from(XML_DECL);
    let _ = write!(
        out,
        "<workbook xmlns=\"{NS_MAIN}\" xmlns:r=\"{NS_DOC_REL}\"><sheets>"
    );
    for (index, sheet) in sheets.iter().enumerate() {
        let n = index + 1;
        out.push_str("<sheet name=\"");
        escape_attr(&mut out, sheet_name(sheet, index));
        let _ = write!(out, "\" sheetId=\"{n}\" r:id=\"rId{n}\"/>");
    }
    out.push_str("</sheets></workbook>");
    out
}

/// A sheet with no name still needs one in the file; Excel's own default is
/// the convention everyone recognises.
fn sheet_name(sheet: &Sheet, index: usize) -> &str {
    if sheet.name.is_empty() {
        match index {
            0 => "Sheet1",
            1 => "Sheet2",
            2 => "Sheet3",
            _ => "Sheet",
        }
    } else {
        &sheet.name
    }
}

fn workbook_rels(sheet_count: usize, has_shared: bool) -> String {
    let mut out = String::from(XML_DECL);
    let _ = write!(out, "<Relationships xmlns=\"{NS_PKG_REL}\">");
    for n in 1..=sheet_count {
        let _ = write!(
            out,
            "<Relationship Id=\"rId{n}\" Type=\"{NS_DOC_REL}/worksheet\" Target=\"worksheets/sheet{n}.xml\"/>"
        );
    }
    let styles_id = sheet_count + 1;
    let _ = write!(
        out,
        "<Relationship Id=\"rId{styles_id}\" Type=\"{NS_DOC_REL}/styles\" Target=\"styles.xml\"/>"
    );
    if has_shared {
        let shared_id = sheet_count + 2;
        let _ = write!(
            out,
            "<Relationship Id=\"rId{shared_id}\" Type=\"{NS_DOC_REL}/sharedStrings\" Target=\"sharedStrings.xml\"/>"
        );
    }
    out.push_str("</Relationships>");
    out
}

/// The reverse of the reader's `builtin_format` table (ECMA-376 §18.8.30): a
/// code that *is* a built-in is written by id, anything else gets a custom
/// `numFmt` from 164 up — the first id ECMA-376 reserves for custom formats.
fn builtin_id(code: &str) -> Option<u32> {
    Some(match code {
        "0" => 1,
        "0.00" => 2,
        "#,##0" => 3,
        "#,##0.00" => 4,
        "0%" => 9,
        "0.00%" => 10,
        "0.00E+00" => 11,
        "# ?/?" => 12,
        "# ??/??" => 13,
        "mm-dd-yy" => 14,
        "d-mmm-yy" => 15,
        "d-mmm" => 16,
        "mmm-yy" => 17,
        "h:mm AM/PM" => 18,
        "h:mm:ss AM/PM" => 19,
        "h:mm" => 20,
        "h:mm:ss" => 21,
        "m/d/yy h:mm" => 22,
        "#,##0 ;(#,##0)" => 37,
        "#,##0 ;[Red](#,##0)" => 38,
        "#,##0.00;(#,##0.00)" => 39,
        "#,##0.00;[Red](#,##0.00)" => 40,
        "mm:ss" => 45,
        "[h]:mm:ss" => 46,
        "mmss.0" => 47,
        "##0.0E+0" => 48,
        "@" => 49,
        _ => return None,
    })
}

/// One `cellXfs` entry as this writer builds it: indices into the three
/// facet tables plus the inline alignment.
///
/// Deduplicated as a tuple, which is the point of `cellXfs` — a thousand cells
/// formatted the same way name one entry (ADR-041 decision 4, at the file
/// boundary).
#[derive(Clone, PartialEq, Eq, Default)]
struct Xf {
    number_format: Option<usize>,
    font: Option<usize>,
    fill: Option<usize>,
    alignment: Option<Alignment>,
}

impl Xf {
    fn is_default(&self) -> bool {
        *self == Xf::default()
    }
}

/// The three facet tables plus the `cellXfs` list, in first-use order.
struct StyleTables {
    formats: Vec<String>,
    fonts: Vec<FontFacet>,
    fills: Vec<u32>,
    xfs: Vec<Xf>,
}

impl StyleTables {
    fn collect(sorted: &[Vec<&Cell>]) -> StyleTables {
        let mut tables = StyleTables {
            formats: Vec::new(),
            fonts: Vec::new(),
            fills: Vec::new(),
            // Index 0 is the default `xf`, so an unformatted cell needs no `s`
            // attribute at all.
            xfs: alloc::vec![Xf::default()],
        };
        for cells in sorted {
            for cell in cells {
                let xf = Xf {
                    number_format: cell
                        .number_format
                        .as_ref()
                        .map(|code| position_or_push(&mut tables.formats, code.clone())),
                    font: cell
                        .font
                        .as_ref()
                        .map(|font| position_or_push(&mut tables.fonts, font.clone())),
                    fill: cell
                        .fill
                        .map(|argb| position_or_push(&mut tables.fills, argb)),
                    alignment: cell.alignment,
                };
                if !xf.is_default() {
                    position_or_push(&mut tables.xfs, xf);
                }
            }
        }
        tables
    }

    /// The `cellXfs` index for a cell, or `None` when it is the default entry
    /// and the `s` attribute can be omitted.
    fn index_of(&self, cell: &Cell) -> Option<usize> {
        if cell.is_unformatted() {
            return None;
        }
        let xf = Xf {
            number_format: cell
                .number_format
                .as_ref()
                .and_then(|code| self.formats.iter().position(|f| f == code)),
            font: cell
                .font
                .as_ref()
                .and_then(|font| self.fonts.iter().position(|f| f == font)),
            fill: cell
                .fill
                .and_then(|argb| self.fills.iter().position(|f| *f == argb)),
            alignment: cell.alignment,
        };
        self.xfs.iter().position(|candidate| *candidate == xf)
    }
}

fn position_or_push<T: PartialEq>(table: &mut Vec<T>, value: T) -> usize {
    match table.iter().position(|existing| *existing == value) {
        Some(at) => at,
        None => {
            table.push(value);
            table.len() - 1
        }
    }
}

/// `styles.xml`, modelling number formats, fonts, fills and alignment — the
/// four facets ADR-041 defines. Borders are still the empty skeleton a
/// conforming consumer requires a styles part to carry, and are claimed as
/// nothing more (**TD-75**).
///
/// Table index 0 is reserved on each axis for the default that every XLSX
/// consumer assumes: font 0 is the workbook font, fill 0 is `none` and fill 1
/// is `gray125` (Excel writes both and some readers index past them), so a
/// modelled font lands at 1.. and a modelled fill at 2...
fn styles_xml(tables: &StyleTables) -> String {
    let formats = &tables.formats;
    let mut out = String::from(XML_DECL);
    let _ = write!(out, "<styleSheet xmlns=\"{NS_MAIN}\">");

    // Ids first: a custom code takes the first free id at or above 164.
    let ids: Vec<u32> = {
        let mut next_custom = 164u32;
        formats
            .iter()
            .map(|code| match builtin_id(code) {
                Some(id) => id,
                None => {
                    let id = next_custom;
                    next_custom += 1;
                    id
                }
            })
            .collect()
    };

    let customs: Vec<(u32, &String)> = formats
        .iter()
        .zip(&ids)
        .filter(|(code, _)| builtin_id(code).is_none())
        .map(|(code, id)| (*id, code))
        .collect();
    if !customs.is_empty() {
        let _ = write!(out, "<numFmts count=\"{}\">", customs.len());
        for (id, code) in &customs {
            let _ = write!(out, "<numFmt numFmtId=\"{id}\" formatCode=\"");
            escape_attr(&mut out, code);
            out.push_str("\"/>");
        }
        out.push_str("</numFmts>");
    }

    let _ = write!(out, "<fonts count=\"{}\">", tables.fonts.len() + 1);
    out.push_str("<font><sz val=\"11\"/><color rgb=\"FF000000\"/><name val=\"Calibri\"/></font>");
    for font in &tables.fonts {
        font_xml(&mut out, font);
    }
    out.push_str("</fonts>");

    let _ = write!(out, "<fills count=\"{}\">", tables.fills.len() + 2);
    out.push_str("<fill><patternFill patternType=\"none\"/></fill>");
    out.push_str("<fill><patternFill patternType=\"gray125\"/></fill>");
    for argb in &tables.fills {
        // `bgColor indexed="64"` is what Excel writes beside a solid fgColor;
        // omitting it makes some consumers render the fill as the pattern
        // background instead of the foreground.
        let _ = write!(
            out,
            "<fill><patternFill patternType=\"solid\"><fgColor rgb=\"{:08X}\"/><bgColor indexed=\"64\"/></patternFill></fill>",
            argb
        );
    }
    out.push_str("</fills>");

    out.push_str(
        "<borders count=\"1\"><border><left/><right/><top/><bottom/><diagonal/></border></borders>",
    );
    out.push_str("<cellStyleXfs count=\"1\"><xf numFmtId=\"0\" fontId=\"0\" fillId=\"0\" borderId=\"0\"/></cellStyleXfs>");

    let _ = write!(out, "<cellXfs count=\"{}\">", tables.xfs.len());
    for xf in &tables.xfs {
        let num_fmt = xf.number_format.map(|i| ids[i]).unwrap_or(0);
        let font_id = xf.font.map(|i| i + 1).unwrap_or(0);
        let fill_id = xf.fill.map(|i| i + 2).unwrap_or(0);
        let _ = write!(
            out,
            "<xf numFmtId=\"{num_fmt}\" fontId=\"{font_id}\" fillId=\"{fill_id}\" borderId=\"0\" xfId=\"0\""
        );
        // The `apply*` flags are what tell Excel the entry's values are the
        // cell's own rather than inherited from its cell style.
        if xf.number_format.is_some() {
            out.push_str(" applyNumberFormat=\"1\"");
        }
        if xf.font.is_some() {
            out.push_str(" applyFont=\"1\"");
        }
        if xf.fill.is_some() {
            out.push_str(" applyFill=\"1\"");
        }
        match &xf.alignment {
            None => out.push_str("/>"),
            Some(a) => {
                out.push_str(" applyAlignment=\"1\">");
                alignment_xml(&mut out, a);
                out.push_str("</xf>");
            }
        }
    }
    out.push_str("</cellXfs>");
    out.push_str("</styleSheet>");
    out
}

fn font_xml(out: &mut String, font: &FontFacet) {
    out.push_str("<font>");
    if font.flags & usk_oplog::FONT_BOLD != 0 {
        out.push_str("<b/>");
    }
    if font.flags & usk_oplog::FONT_ITALIC != 0 {
        out.push_str("<i/>");
    }
    if font.flags & usk_oplog::FONT_STRIKE != 0 {
        out.push_str("<strike/>");
    }
    if font.flags & usk_oplog::FONT_UNDERLINE != 0 {
        out.push_str("<u/>");
    }
    // Half-points back to points. A whole number prints without a fraction, so
    // the common 11 does not become "11.0" and fail a byte comparison against
    // what Excel writes.
    if font.half_points.is_multiple_of(2) {
        let _ = write!(out, "<sz val=\"{}\"/>", font.half_points / 2);
    } else {
        let _ = write!(out, "<sz val=\"{}.5\"/>", font.half_points / 2);
    }
    let _ = write!(out, "<color rgb=\"{:08X}\"/><name val=\"", font.argb);
    escape_attr(out, &font.name);
    out.push_str("\"/></font>");
}

fn alignment_xml(out: &mut String, a: &Alignment) {
    out.push_str("<alignment");
    match a.horizontal {
        1 => out.push_str(" horizontal=\"left\""),
        2 => out.push_str(" horizontal=\"center\""),
        3 => out.push_str(" horizontal=\"right\""),
        _ => {}
    }
    match a.vertical {
        1 => out.push_str(" vertical=\"top\""),
        2 => out.push_str(" vertical=\"center\""),
        _ => {}
    }
    if a.wrap {
        out.push_str(" wrapText=\"1\"");
    }
    out.push_str("/>");
}

fn shared_strings_xml(shared: &[String], sorted: &[Vec<&Cell>]) -> String {
    // `count` is total references, `uniqueCount` the table size; Excel writes
    // both and readers that trust either should find them true.
    let references: usize = sorted
        .iter()
        .flat_map(|cells| cells.iter())
        .filter(|c| c.formula.is_none() && matches!(c.value, Value::Text(_)))
        .count();
    let mut out = String::from(XML_DECL);
    let _ = write!(
        out,
        "<sst xmlns=\"{NS_MAIN}\" count=\"{references}\" uniqueCount=\"{}\">",
        shared.len()
    );
    for text in shared {
        out.push_str("<si><t");
        if needs_space_preserve(text) {
            out.push_str(" xml:space=\"preserve\"");
        }
        out.push('>');
        escape_text(&mut out, text);
        out.push_str("</t></si>");
    }
    out.push_str("</sst>");
    out
}

/// Leading/trailing whitespace and newlines are exactly what a whitespace-
/// normalising consumer would eat; `xml:space="preserve"` is the spec's way of
/// saying "do not".
fn needs_space_preserve(text: &str) -> bool {
    text.starts_with(char::is_whitespace)
        || text.ends_with(char::is_whitespace)
        || text.contains('\n')
}

fn sheet_xml(
    sheet: &Sheet,
    cells: &[&Cell],
    styles: &StyleTables,
    shared: &[String],
    report: &mut WriteReport,
) -> String {
    let mut out = String::from(XML_DECL);
    let _ = write!(out, "<worksheet xmlns=\"{NS_MAIN}\"><sheetData>");

    let mut row_open: Option<u32> = None;
    for cell in cells {
        if row_open != Some(cell.row) {
            if row_open.is_some() {
                out.push_str("</row>");
            }
            let _ = write!(out, "<row r=\"{}\">", cell.row + 1);
            row_open = Some(cell.row);
        }
        emit_cell(&mut out, sheet, cell, styles, shared, report);
    }
    if row_open.is_some() {
        out.push_str("</row>");
    }
    out.push_str("</sheetData></worksheet>");
    out
}

fn emit_cell(
    out: &mut String,
    sheet: &Sheet,
    cell: &Cell,
    styles: &StyleTables,
    shared: &[String],
    report: &mut WriteReport,
) {
    let reference = render_a1(&A1 {
        row: cell.row,
        col: cell.col,
        row_absolute: false,
        col_absolute: false,
    });

    // `s` names a `cellXfs` index; entry 0 is the all-defaults one, so an
    // unformatted cell carries no `s` at all.
    let style = styles.index_of(cell);
    if cell.number_format.is_some() {
        report.number_formats_written += 1;
    }
    if style.is_some() {
        report.styles_written += 1;
    }
    report.fonts_written += usize::from(cell.font.is_some());
    report.fills_written += usize::from(cell.fill.is_some());
    report.alignments_written += usize::from(cell.alignment.is_some());

    // The cell's type attribute and `<v>` body, decided before anything is
    // written so a loss is recorded exactly once.
    let (type_attr, body) = match &cell.value {
        Value::Blank => (None, None),
        Value::Bool(b) => (
            Some("b"),
            Some(BodyText::Raw(if *b { "1" } else { "0" }.to_string())),
        ),
        Value::Number(n) if n.is_finite() => (None, Some(BodyText::Raw(render_number(*n)))),
        Value::Number(_) => {
            // NaN / ±inf: no XLSX spelling exists. Excel stores #NUM! for an
            // overflowed result; matching that beats inventing one.
            report.losses.push(WriteLoss {
                sheet: sheet.name.clone(),
                reference: reference.clone(),
                reason: WriteLossReason::NonFiniteNumber,
            });
            (
                Some("e"),
                Some(BodyText::Raw(ErrorKind::Num.as_str().to_string())),
            )
        }
        Value::Decimal(d) => {
            // The exact digits go in the file; every XLSX reader will parse
            // them as a double, so the *type* is recorded as lost.
            report.losses.push(WriteLoss {
                sheet: sheet.name.clone(),
                reference: reference.clone(),
                reason: WriteLossReason::DecimalWrittenAsNumber,
            });
            let mut text = String::new();
            let _ = write!(text, "{d}");
            (None, Some(BodyText::Raw(text)))
        }
        Value::Text(text) => {
            if cell.formula.is_some() {
                // A formula whose cached result is text: `t="str"`, inline.
                (Some("str"), Some(BodyText::Escaped(text.clone())))
            } else {
                let index = shared.iter().position(|s| s == text).unwrap_or_default();
                let mut v = String::new();
                let _ = write!(v, "{index}");
                (Some("s"), Some(BodyText::Raw(v)))
            }
        }
        Value::Error(e) => {
            let spelling = match e.kind {
                ErrorKind::Circ | ErrorKind::Spill => {
                    report.losses.push(WriteLoss {
                        sheet: sheet.name.clone(),
                        reference: reference.clone(),
                        reason: WriteLossReason::ErrorOutsideXlsxVocabulary,
                    });
                    ErrorKind::Na.as_str()
                }
                kind => kind.as_str(),
            };
            (Some("e"), Some(BodyText::Raw(spelling.to_string())))
        }
    };

    let _ = write!(out, "<c r=\"{reference}\"");
    if let Some(index) = style {
        let _ = write!(out, " s=\"{index}\"");
    }
    if let Some(t) = type_attr {
        let _ = write!(out, " t=\"{t}\"");
    }
    if body.is_none() && cell.formula.is_none() {
        out.push_str("/>");
        report.cells_written += 1;
        return;
    }
    out.push('>');
    if let Some(formula) = &cell.formula {
        out.push_str("<f>");
        escape_text(out, formula);
        out.push_str("</f>");
        report.formulas_written += 1;
    }
    match body {
        Some(BodyText::Raw(text)) => {
            out.push_str("<v>");
            out.push_str(&text);
            out.push_str("</v>");
        }
        Some(BodyText::Escaped(text)) => {
            out.push_str("<v>");
            escape_text(out, &text);
            out.push_str("</v>");
        }
        None => {}
    }
    out.push_str("</c>");
    report.cells_written += 1;
}

/// A `<v>` body is either machine text this module produced (numbers, error
/// spellings, indices — nothing to escape) or user text, which must be.
enum BodyText {
    Raw(String),
    Escaped(String),
}

/// The canonical decimal rendering — the same rule as `usk_csv::writer`'s,
/// restated here because the two crates do not depend on each other: integers
/// under 10^15 print without a fraction, everything else prints Rust's
/// shortest representation that parses back to the identical double. The
/// round-trip tests hold `v.parse::<f64>()` to bit-equality.
///
/// The caller handles non-finite values; this function only sees finite ones.
fn render_number(n: f64) -> String {
    let mut out = String::new();
    if n.abs() < 1e15 && n == (n as i64) as f64 {
        let _ = write!(out, "{}", n as i64);
    } else {
        let _ = write!(out, "{n:?}");
    }
    out
}

/// Text-content escaping: the two characters XML cannot carry raw, plus `>`
/// for symmetry, plus numeric references for the C0 controls XML 1.0 cannot
/// carry raw either. Tab, LF and CR pass through in text content.
///
/// A control character below 0x20 (other than tab/LF/CR) is strictly outside
/// XML 1.0's character range even as a reference; our own reader accepts the
/// reference, Excel may not. Recorded with the writer's other honesty notes
/// rather than silently stripped — dropping a byte of the user's text is a
/// worse failure than writing a file a stricter parser rejects.
fn escape_text(out: &mut String, text: &str) {
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            c if (c as u32) < 0x20 && c != '\t' && c != '\n' && c != '\r' => {
                let _ = write!(out, "&#x{:X};", c as u32);
            }
            c => out.push(c),
        }
    }
}

/// Attribute escaping: text escaping plus the quote, plus tab/LF/CR — which
/// are legal in an attribute but subject to whitespace normalisation, so the
/// reference form is the only one that survives verbatim.
fn escape_attr(out: &mut String, text: &str) {
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "&#x{:X};", c as u32);
            }
            c => out.push(c),
        }
    }
}
