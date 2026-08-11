//! usk-xlsx — XLSX **read and write** (BOOTSTRAP row 12 + session 29, docs/24).
//!
//! > *XLSX read (values+formulas) in a sandboxed subprocess ... per-file
//! > fidelity reports on legacy imports; the number is published.*
//!
//! Values, formulas and number formats, out of the parts that carry them.
//! `no_std + alloc`, no I/O: it is handed a container's bytes by
//! `ehkatra-parse`, which is the process that may safely be wrong about them.
//!
//! # A projection, in both directions
//! BOOTSTRAP put XLSX *write* outside v0.1, so the read side is deliberately
//! a projection and not a lossless model. The parts that are not read are
//! **named in the report** rather than silently dropped — a fidelity number
//! that counts only what it looked at is not a fidelity number.
//!
//! The [`write`] module (session 29) is that projection's inverse: it emits
//! exactly the modelled surface — values, formulas with cached results,
//! number formats — and holds itself to the same honesty rule via
//! [`write::WriteReport`]: source parts not re-emitted and cells that could
//! not cross losslessly are named, never silently dropped. Round-trip
//! read → write → re-read over the corpus is the published write-fidelity
//! number (MEASUREMENTS.md, W-XLSX-WRITE).
//!
//! # Active content (docs/24)
//! > *active (vbaProject, OLE, ActiveX, DDE) → quarantine ... never executed,
//! > never re-emitted by default*
//!
//! Nothing here executes anything — there is no interpreter to reach — but
//! "we happened not to run it" is not the same claim as "we found it and set it
//! aside". Active parts are detected by name, listed in the report, and their
//! bytes are never decompressed.

#![no_std]
extern crate alloc;

mod parse;
pub mod write;

use alloc::string::String;
use alloc::vec::Vec;
use usk_types::Value;
use usk_zip::ZipError;

pub use parse::read;

/// One cell, as XLSX describes it.
#[derive(Clone, PartialEq, Debug)]
pub struct Cell {
    /// 0-based, so it lines up with the engine's view ordinals.
    pub row: u32,
    pub col: u32,
    pub value: Value,
    /// The formula **as authored**, without its leading `=` (XLSX stores it
    /// that way). `None` for a literal cell.
    ///
    /// The stored *value* is kept alongside rather than recomputed: it is what
    /// Excel last calculated, and comparing it against our own evaluation is
    /// how the conformance story and the fidelity report get their evidence.
    pub formula: Option<String>,
    /// The resolved number-format code (`"0.00"`, `"yyyy-mm-dd"`), if the cell
    /// has a style that names one.
    pub number_format: Option<String>,
}

#[derive(Clone, PartialEq, Debug)]
pub struct Sheet {
    pub name: String,
    /// The part this sheet came from, so a fidelity report can name it.
    pub part: String,
    pub cells: Vec<Cell>,
}

impl Sheet {
    pub fn cell(&self, row: u32, col: u32) -> Option<&Cell> {
        self.cells.iter().find(|c| c.row == row && c.col == col)
    }
}

/// What one file gave up, and what it did not. **This is the deliverable**, not
/// a diagnostic: docs/24 makes fidelity a published, measured attribute.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct Fidelity {
    /// Parts present in the container.
    pub parts_total: usize,
    /// Parts this build reads.
    pub parts_read: Vec<String>,
    /// Parts present and understood to exist, but not modelled in v0.1 —
    /// charts, drawings, pivot caches, themes. Named individually so the number
    /// can be argued with.
    pub parts_ignored: Vec<String>,
    /// docs/24's active-content class. Never decompressed, never executed.
    pub quarantined: Vec<String>,
    /// Package plumbing that carries no user data — the content-type map and
    /// the root relationship. Tracked separately from [`Fidelity::parts_ignored`]
    /// because *not reading them loses nothing*, and folding them into the
    /// coverage ratio would make a workbook we read perfectly score 60%.
    pub parts_structural: Vec<String>,
    pub cells_read: usize,
    pub formulas_read: usize,
    /// Cells carrying a number format this build resolved to a code.
    pub number_formats_resolved: usize,
    /// Things that were read but lost something on the way — an unsupported
    /// cell type, an unresolvable style. Each is a fidelity miss with a reason.
    pub losses: Vec<Loss>,
}

impl Fidelity {
    /// Parts read ÷ parts that **carry user data and are safe to read**.
    ///
    /// Two exclusions from the denominator, each for a different reason, and
    /// both of which the first version of this function got wrong by lumping
    /// them in:
    /// * **Quarantined** parts (docs/24's active content). Not reading
    ///   `vbaProject.bin` is the *correct* outcome; counting it as a miss would
    ///   push the number down for doing the right thing.
    /// * **Structural** parts — the content-type map and the root relationship.
    ///   They carry no user data, so not reading them loses nothing. Leaving
    ///   them in scored a workbook we read perfectly at 60%, which is not a
    ///   fidelity number, it is noise.
    ///
    /// What stays in the denominator is [`Fidelity::parts_ignored`] — charts,
    /// drawings, pivot caches. Those *are* user data this build drops, and a
    /// coverage number that excused them would be measuring its own scope
    /// rather than the file.
    pub fn part_coverage(&self) -> f64 {
        let considered = self
            .parts_total
            .saturating_sub(self.quarantined.len())
            .saturating_sub(self.parts_structural.len());
        if considered == 0 {
            return 100.0;
        }
        100.0 * self.parts_read.len() as f64 / considered as f64
    }

    pub fn is_lossless(&self) -> bool {
        self.losses.is_empty() && self.parts_ignored.is_empty()
    }
}

/// A specific thing that did not survive the read.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Loss {
    pub part: String,
    pub reference: String,
    pub reason: LossReason,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LossReason {
    /// A `t=` cell type this build does not model.
    UnsupportedCellType,
    /// A style index with no entry in `styles.xml`.
    UnresolvedStyle,
    /// A shared-string index past the end of the table.
    SharedStringOutOfRange,
    /// A cell reference that is not A1-shaped.
    UnparseableReference,
    /// A `<v>` that is not a number where the type says it should be.
    UnparseableValue,
}

/// A workbook, plus the honest account of reading it.
#[derive(Clone, PartialEq, Debug)]
pub struct Workbook {
    pub sheets: Vec<Sheet>,
    pub fidelity: Fidelity,
}

/// Why a container is not an XLSX we can read.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum XlsxError {
    Zip(ZipError),
    /// No `xl/workbook.xml`. A ZIP, but not a spreadsheet.
    NotAWorkbook,
    /// A part failed to parse. Carries the part so the report can name it.
    BadPart {
        part: String,
        detail: String,
    },
}

impl From<ZipError> for XlsxError {
    fn from(err: ZipError) -> Self {
        XlsxError::Zip(err)
    }
}

/// docs/24's **active** ingest class: quarantined, never executed, never
/// re-emitted.
///
/// Matched on the part name because that is what the format guarantees; a
/// content-type sniff would be cleverer and would disagree with Excel, which
/// dispatches on exactly these names.
pub fn is_active_content(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.ends_with(".bin") && lower.contains("vba")
        || lower.contains("vbaproject")
        || lower.contains("olebject")
        || lower.contains("oleobject")
        || lower.contains("activex")
        || lower.ends_with(".dll")
        || lower.ends_with(".exe")
}

/// Parts v0.1 knows exist and does not model. Listed rather than lumped into
/// "everything else", so the fidelity report distinguishes *"we ignored the
/// chart"* from *"we did not recognise this at all"*.
pub fn is_known_unmodelled(name: &str) -> bool {
    const PREFIXES: [&str; 8] = [
        "xl/charts/",
        "xl/drawings/",
        "xl/pivotCache/",
        "xl/pivotTables/",
        "xl/theme/",
        "xl/media/",
        "xl/tables/",
        "customXml/",
    ];
    PREFIXES.iter().any(|prefix| name.starts_with(prefix))
        || name.ends_with(".vml")
        || name.starts_with("docProps/")
}

/// Package plumbing: present in every container, carrying no user data.
///
/// v0.1 reaches `xl/workbook.xml` by convention rather than by resolving the
/// root relationship, so these are genuinely unread — but unread plumbing is
/// not lost data, and [`Fidelity::part_coverage`] excludes them for that
/// reason. They are still *listed*, so the report never has a part it cannot
/// account for.
pub fn is_package_plumbing(name: &str) -> bool {
    name == "[Content_Types].xml" || name == "_rels/.rels"
}
