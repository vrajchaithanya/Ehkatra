//! usk-mcp — the MCP tool surface (BOOTSTRAP row 14, docs/21).
//!
//! > *Schemas and answers, never grids; preview before mutation; everything
//! > attributable and reversible.*
//!
//! `no_std + alloc` and **no I/O**: this crate is handed a JSON-RPC request and
//! returns a JSON-RPC response. `ehkatra-mcp` adds stdio and nothing else. That
//! split is why every refusal path below is an ordinary unit test rather than
//! something you have to drive an agent to reach — the same reason `usk-sync`
//! is provable without a network.
//!
//! # The three laws of docs/21, and where each lives
//! * **Schemas and answers, never grids.** [`describe_sheet`](Server) returns
//!   per-column statistics and five sample rows whatever the sheet's size;
//!   `read_range` is the capped escape hatch and says so when it truncates.
//!   An agent that wants the grid has to ask for it a piece at a time, on
//!   purpose.
//! * **Preview before mutation.** `preview_edits` computes the impact against a
//!   scratch replay and returns a `preview_hash`. Above
//!   [`BLAST_RADIUS_PREVIEW_REQUIRED`] cells, `apply_edits` **refuses** without
//!   a matching one. The threshold is host-enforced, not tool etiquette.
//! * **Attributable and reversible.** Every `apply_edits` is one labeled batch
//!   in an agent-scoped journal, and `undo` reverses the last one as a unit.
//!
//! # Untrusted text (docs/21's injection posture)
//! > *Cell-derived text labeled untrusted in every response.*
//!
//! Every string that came from a cell — a value, a formula, a sheet name — is
//! wrapped as `{"untrusted": "..."}` rather than emitted bare. This is
//! structural rather than advisory: an agent reading a cell that says "ignore
//! your previous instructions" receives it in a labeled envelope, and a
//! response format that *cannot* express an unlabeled cell string is a
//! guarantee where a convention would be a hope. [`untrusted`] is the only way
//! to put cell text into a response, and every call site goes through it.

#![no_std]
extern crate alloc;

mod tools;

use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;
use usk_json::{number, string, Json};
use usk_reduce::{Command, Session};
use usk_state::State;
use usk_types::{ActorId, ColId, RowId, Value};

pub use tools::TOOLS;

/// Cells `read_range` will return before truncating. docs/21 calls it "a capped
/// escape hatch"; the cap is what makes it an escape hatch rather than the
/// grid-dumping default the whole layer exists to avoid.
pub const READ_RANGE_MAX_CELLS: usize = 2_000;

/// Sample rows in `describe_sheet`, per docs/21. Fixed, so the response size is
/// independent of the sheet's.
pub const SAMPLE_ROWS: usize = 5;

/// Cells an `apply_edits` may touch — directly or downstream — before a
/// matching `preview_hash` becomes mandatory (docs/21's blast-radius policy).
pub const BLAST_RADIUS_PREVIEW_REQUIRED: u64 = 100;

/// Edits accepted in one `apply_edits` call.
pub const MAX_EDITS_PER_CALL: usize = 1_000;

/// Rows or columns one call may grow the sheet by.
///
/// Auto-growth makes the first edit to an empty workbook work; unbounded
/// auto-growth makes `{"cell":"XFD1048576","value":1}` a denial of service that
/// costs the agent one line. The bound is what separates the two.
pub const MAX_GROWTH_PER_CALL: u32 = 4_096;

/// The MCP protocol revision this server implements.
pub const PROTOCOL_VERSION: &str = "2024-11-05";

/// Wraps cell-derived text in its untrusted envelope.
///
/// **The only route for user-controlled text into a response.** Keeping it a
/// single function is the point: a reviewer can grep for the call sites and see
/// that every one of them is a cell, a formula or a sheet name.
pub fn untrusted(text: &str) -> Json {
    Json::Object(vec![(String::from("untrusted"), string(text))])
}

/// One agent-applied batch, for the agent-scoped undo docs/21 requires.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Batch {
    pub label: String,
    /// Reducer undo groups this batch pushed. `undo` pops exactly this many, so
    /// a batch is reversed as a unit rather than one edit at a time.
    pub groups: usize,
}

/// A computed preview, held until it is used or replaced.
#[derive(Clone, PartialEq, Eq, Debug)]
struct Preview {
    hash: String,
    cells_changed: u64,
    downstream: u64,
    errors_introduced: u64,
}

/// The server: a workbook, an agent's batch journal, and the last preview.
pub struct Server {
    session: Session,
    batches: Vec<Batch>,
    preview: Option<Preview>,
}

impl Server {
    pub fn new(actor: ActorId) -> Server {
        Server {
            session: Session::new(actor),
            batches: Vec::new(),
            preview: None,
        }
    }

    /// The workbook, for a host that needs to seed or inspect it.
    pub fn session(&mut self) -> &mut Session {
        &mut self.session
    }

    pub fn batches(&self) -> &[Batch] {
        &self.batches
    }

    /// Handles one JSON-RPC request.
    ///
    /// Returns `None` for a notification (no `id`), which JSON-RPC says must
    /// not be answered — a server that replies to a notification desynchronises
    /// a client that is not expecting one.
    pub fn handle(&mut self, request: &Json) -> Option<Json> {
        let id = request.get("id").cloned();
        let method = request.get("method").and_then(Json::as_str).unwrap_or("");
        let params = request.get("params");

        let result = match method {
            "initialize" => Ok(self.initialize()),
            "tools/list" => Ok(tools::list()),
            "tools/call" => self.call_tool(params),
            "ping" => Ok(Json::Object(Vec::new())),
            other => Err(RpcError::method_not_found(other)),
        };

        let id = id?;
        Some(match result {
            Ok(value) => Json::Object(vec![
                (String::from("jsonrpc"), string("2.0")),
                (String::from("id"), id),
                (String::from("result"), value),
            ]),
            Err(err) => Json::Object(vec![
                (String::from("jsonrpc"), string("2.0")),
                (String::from("id"), id),
                (String::from("error"), err.to_json()),
            ]),
        })
    }

    fn initialize(&self) -> Json {
        Json::Object(vec![
            (String::from("protocolVersion"), string(PROTOCOL_VERSION)),
            (
                String::from("capabilities"),
                Json::Object(vec![(String::from("tools"), Json::Object(Vec::new()))]),
            ),
            (
                String::from("serverInfo"),
                Json::Object(vec![
                    (String::from("name"), string("ehkatra")),
                    (String::from("version"), string(env!("CARGO_PKG_VERSION"))),
                ]),
            ),
        ])
    }

    fn call_tool(&mut self, params: Option<&Json>) -> Result<Json, RpcError> {
        let params = params.ok_or_else(|| RpcError::invalid_params("params required"))?;
        let name = params
            .get("name")
            .and_then(Json::as_str)
            .ok_or_else(|| RpcError::invalid_params("name required"))?;
        let empty = Json::Object(Vec::new());
        let args = params.get("arguments").unwrap_or(&empty);

        let outcome = match name {
            "describe_workbook" => Ok(self.describe_workbook()),
            "describe_sheet" => Ok(self.describe_sheet()),
            "read_range" => self.read_range(args),
            "explain_cell" => self.explain_cell(args),
            "preview_edits" => self.preview_edits(args),
            "apply_edits" => self.apply_edits(args),
            "undo" => Ok(self.undo()),
            other => return Err(RpcError::unknown_tool(other)),
        };

        // A *tool* failure is a result with `isError`, not a JSON-RPC error:
        // the call succeeded, the tool declined. Conflating the two makes an
        // agent unable to tell "you asked wrongly" from "the server broke".
        Ok(match outcome {
            Ok(value) => tool_result(value, false),
            Err(refusal) => tool_result(refusal.to_json(), true),
        })
    }

    // ------------------------------------------------------------- orient

    fn describe_workbook(&mut self) -> Json {
        let version = self.version();
        let state = self.session.state();
        let rows = state.row_order().len();
        let cols = state.col_order().len();
        let formulas = state.formulas().count();
        let filled = count_filled(state);

        Json::Object(vec![
            (String::from("version"), version),
            (
                String::from("sheets"),
                Json::Array(vec![Json::Object(vec![
                    // v0.1's workbook is one grid (BOOTSTRAP row 3 onwards).
                    // Reporting a single sheet by name rather than inventing a
                    // sheet model keeps the tool honest about what exists.
                    (String::from("name"), untrusted("Sheet1")),
                    (String::from("rows"), number(rows as f64)),
                    (String::from("columns"), number(cols as f64)),
                ])]),
            ),
            (String::from("cells_filled"), number(filled as f64)),
            (String::from("formulas"), number(formulas as f64)),
        ])
    }

    /// Per-column statistics and a bounded sample — docs/21's *"bounded at any
    /// scale"*. The response size does not depend on the sheet's.
    fn describe_sheet(&mut self) -> Json {
        let version = self.version();
        self.session.settle();
        let rows: Vec<RowId> = self.session.state().row_order();
        let cols: Vec<ColId> = self.session.state().col_order();

        let mut columns = Vec::with_capacity(cols.len());
        for (index, col) in cols.iter().enumerate() {
            let mut blank = 0usize;
            let mut numeric = 0usize;
            let mut text = 0usize;
            let mut logical = 0usize;
            let mut errors = 0usize;
            let mut formulas = 0usize;
            for row in &rows {
                if self.session.state().formula(*row, *col).is_some() {
                    formulas += 1;
                }
                match self.session.value(*row, *col) {
                    None | Some(Value::Blank) => blank += 1,
                    Some(Value::Number(_)) | Some(Value::Decimal(_)) => numeric += 1,
                    Some(Value::Text(_)) => text += 1,
                    Some(Value::Bool(_)) => logical += 1,
                    Some(Value::Error(_)) => errors += 1,
                }
            }
            columns.push(Json::Object(vec![
                (String::from("column"), string(column_name(index as u32))),
                (String::from("blank"), number(blank as f64)),
                (String::from("numeric"), number(numeric as f64)),
                (String::from("text"), number(text as f64)),
                (String::from("logical"), number(logical as f64)),
                (String::from("errors"), number(errors as f64)),
                (String::from("formulas"), number(formulas as f64)),
            ]));
        }

        let mut samples = Vec::new();
        for (index, row) in rows.iter().take(SAMPLE_ROWS).enumerate() {
            let cells: Vec<Json> = cols.iter().map(|col| self.value_json(*row, *col)).collect();
            samples.push(Json::Object(vec![
                (String::from("row"), number(index as f64 + 1.0)),
                (String::from("cells"), Json::Array(cells)),
            ]));
        }

        Json::Object(vec![
            (String::from("version"), version),
            (String::from("name"), untrusted("Sheet1")),
            (String::from("rows"), number(rows.len() as f64)),
            (String::from("columns"), Json::Array(columns)),
            (String::from("sample_rows"), Json::Array(samples)),
            (
                String::from("sample_truncated"),
                Json::Bool(rows.len() > SAMPLE_ROWS),
            ),
        ])
    }

    fn read_range(&mut self, args: &Json) -> Result<Json, Refusal> {
        let rect = Ordinals::from_args(args)?;
        self.session.settle();
        let rows = self.session.state().row_order();
        let cols = self.session.state().col_order();

        let requested = rect.cell_count();
        let mut out = Vec::new();
        let mut returned = 0usize;
        'outer: for r in rect.row0..=rect.row1 {
            let Some(row) = rows.get(r as usize) else {
                break;
            };
            let mut line = Vec::new();
            for c in rect.col0..=rect.col1 {
                let Some(col) = cols.get(c as usize) else {
                    break;
                };
                if returned >= READ_RANGE_MAX_CELLS {
                    out.push(Json::Array(line));
                    break 'outer;
                }
                line.push(self.value_json(*row, *col));
                returned += 1;
            }
            out.push(Json::Array(line));
        }

        Ok(Json::Object(vec![
            (String::from("cells"), Json::Array(out)),
            (String::from("cells_returned"), number(returned as f64)),
            (String::from("cells_requested"), number(requested as f64)),
            // Truncation is stated, never inferred from a short array. An agent
            // that cannot tell a small answer from a clipped one will act on
            // the clipped one (docs/21: "capped with explicit truncation").
            (
                String::from("truncated"),
                Json::Bool(returned < requested as usize),
            ),
            (String::from("cap"), number(READ_RANGE_MAX_CELLS as f64)),
        ]))
    }

    /// docs/21's `explain_formula` + `trace_error`, joined: what the cell holds,
    /// what it computed, and — when it is an error — where the error came from.
    fn explain_cell(&mut self, args: &Json) -> Result<Json, Refusal> {
        let (r, c) = Ordinals::cell_from_args(args)?;
        self.session.settle();
        let row = *self
            .session
            .state()
            .row_order()
            .get(r as usize)
            .ok_or_else(|| Refusal::out_of_range("row"))?;
        let col = *self
            .session
            .state()
            .col_order()
            .get(c as usize)
            .ok_or_else(|| Refusal::out_of_range("column"))?;

        let formula = self
            .session
            .state()
            .formula(row, col)
            .map(|f| f.source.clone());
        let value = self.session.value(row, col);

        let mut fields = vec![
            (String::from("reference"), string(a1(r, c))),
            (String::from("value"), value_json(value.as_ref())),
            (
                String::from("kind"),
                string(if formula.is_some() {
                    "formula"
                } else {
                    "literal"
                }),
            ),
        ];
        if let Some(source) = &formula {
            fields.push((String::from("formula"), untrusted(source)));
        }
        // The origin trace is the differentiator: docs/06's "every #VALUE!
        // answers where did you come from". It is carried on the value itself
        // (`Origin`), so this is a projection rather than a reconstruction.
        if let Some(Value::Error(err)) = &value {
            fields.push((
                String::from("error"),
                Json::Object(vec![
                    (String::from("kind"), string(err.kind.as_str())),
                    (String::from("origin"), string(origin_text(&err.origin))),
                ]),
            ));
        }
        Ok(Json::Object(fields))
    }

    // ------------------------------------------------------------- mutate

    fn preview_edits(&mut self, args: &Json) -> Result<Json, Refusal> {
        let edits = parse_edits(args)?;
        let mut batch = self.growth(&edits)?;
        batch.extend(edits.iter().cloned());
        // Validated against *current* state before anything is simulated, so a
        // preview never describes a batch that could not be applied.
        self.validate(&batch)?;

        let before = self.state_hash();
        let mut scratch = Session::new(self.session.actor());
        scratch.integrate_batch(self.session.log.ops().to_vec());
        let before_values = snapshot(&mut scratch);

        for edit in &batch {
            scratch
                .apply(edit.clone())
                .map_err(|e| Refusal::command(e, "preview"))?;
        }
        let after_values = snapshot(&mut scratch);
        let hash = hex(scratch.state().state_hash().as_bytes());

        let direct = edits.len() as u64;
        let mut changed = 0u64;
        let mut errors_introduced = 0u64;
        for (key, after) in &after_values {
            let was = before_values.iter().find(|(k, _)| k == key).map(|(_, v)| v);
            if was != Some(after) {
                changed += 1;
                let was_error = matches!(was, Some(Value::Error(_)));
                if matches!(after, Value::Error(_)) && !was_error {
                    errors_introduced += 1;
                }
            }
        }
        let downstream = changed.saturating_sub(direct);

        self.preview = Some(Preview {
            hash: hash.clone(),
            cells_changed: changed,
            downstream,
            errors_introduced,
        });

        Ok(Json::Object(vec![
            (String::from("preview_hash"), string(&hash)),
            (String::from("base_version"), string(&before)),
            (
                String::from("impact"),
                Json::Object(vec![
                    (String::from("cells_edited"), number(direct as f64)),
                    (String::from("cells_changed"), number(changed as f64)),
                    (
                        String::from("downstream_changed"),
                        number(downstream as f64),
                    ),
                    (
                        String::from("errors_introduced"),
                        number(errors_introduced as f64),
                    ),
                ]),
            ),
            (
                String::from("preview_required_to_apply"),
                Json::Bool(changed > BLAST_RADIUS_PREVIEW_REQUIRED),
            ),
        ]))
    }

    fn apply_edits(&mut self, args: &Json) -> Result<Json, Refusal> {
        let edits = parse_edits(args)?;
        let label = args
            .get("label")
            .and_then(Json::as_str)
            .unwrap_or("agent edit")
            .to_string();

        // Optimistic concurrency: the agent states the version it reasoned
        // about, and a workbook that has moved since refuses rather than
        // applying an edit to a world the agent never saw.
        if let Some(expected) = args.get("expected_version").and_then(Json::as_str) {
            let actual = self.state_hash();
            if expected != actual {
                return Err(Refusal::version_mismatch(expected, &actual));
            }
        }

        let growth = self.growth(&edits)?;
        let mut batch = growth.clone();
        batch.extend(edits.iter().cloned());
        self.validate(&batch)?;

        // docs/21's blast-radius policy, host-enforced. A large edit must have
        // been previewed, and the preview must be *this* edit — the hash is
        // what ties the two together, and without it "I previewed something"
        // would be enough.
        let preview = self.preview.clone();
        let supplied = args.get("preview_hash").and_then(Json::as_str);
        let large = preview
            .as_ref()
            .map(|p| p.cells_changed > BLAST_RADIUS_PREVIEW_REQUIRED)
            .unwrap_or(false);
        if large {
            match (supplied, preview.as_ref()) {
                (Some(given), Some(p)) if given == p.hash => {}
                (Some(given), Some(p)) => return Err(Refusal::preview_mismatch(given, &p.hash)),
                _ => return Err(Refusal::preview_required(BLAST_RADIUS_PREVIEW_REQUIRED)),
            }
        } else if let (Some(given), Some(p)) = (supplied, preview.as_ref()) {
            // A supplied hash is always checked, even when not required: an
            // agent that offers evidence gets it verified rather than ignored.
            if given != p.hash {
                return Err(Refusal::preview_mismatch(given, &p.hash));
            }
        }

        let mut groups = 0usize;
        let mut ops = 0usize;
        for edit in &batch {
            let report = self
                .session
                .apply(edit.clone())
                .map_err(|e| Refusal::command(e, "apply"))?;
            groups += 1;
            ops += report.ops_emitted;
        }
        self.batches.push(Batch {
            label: label.clone(),
            groups,
        });
        self.preview = None;

        let rows_added = growth
            .iter()
            .filter(|c| matches!(c, Command::InsertRow { .. }))
            .count();
        let columns_added = growth.len() - rows_added;
        Ok(Json::Object(vec![
            (String::from("applied"), number(edits.len() as f64)),
            (String::from("ops_emitted"), number(ops as f64)),
            // Growth is reported, never silent: an agent that did not realise
            // it was extending the sheet should find out from the response.
            (String::from("rows_added"), number(rows_added as f64)),
            (String::from("columns_added"), number(columns_added as f64)),
            (String::from("label"), untrusted(&label)),
            (String::from("version"), self.version()),
            (String::from("undo_available"), Json::Bool(true)),
        ]))
    }

    /// Reverses the last agent batch **as a unit** (docs/21: agent-session
    /// scoped undo).
    fn undo(&mut self) -> Json {
        let Some(batch) = self.batches.pop() else {
            return Json::Object(vec![
                (String::from("undone"), Json::Bool(false)),
                (String::from("reason"), string("no agent batch to undo")),
                (String::from("version"), self.version()),
            ]);
        };
        let mut blocked = 0usize;
        for _ in 0..batch.groups {
            let report = self.session.apply(Command::Undo).unwrap_or_default();
            blocked += report.blocked;
        }
        Json::Object(vec![
            (String::from("undone"), Json::Bool(true)),
            (String::from("label"), untrusted(&batch.label)),
            (String::from("groups"), number(batch.groups as f64)),
            // docs/11's blocked-and-narrowed, surfaced rather than silent: an
            // undo that could not restore a cell because a collaborator now
            // owns it says so.
            (String::from("blocked"), number(blocked as f64)),
            (String::from("version"), self.version()),
        ])
    }

    // ------------------------------------------------------------ helpers

    /// The `InsertRow`/`InsertCol` commands an edit batch needs before its
    /// cells exist.
    ///
    /// An agent writing to `A1` of an empty workbook is the *ordinary* first
    /// edit, and refusing it would make the surface unusable at exactly the
    /// moment an agent starts working — which is what the end-to-end test found
    /// on its first run. A spreadsheet grows when you type outside it, and so
    /// does this.
    ///
    /// The growth is emitted as ordinary ops in the same batch, so it is
    /// attributable, replayable and undone with the rest of it (DP-A1). It is
    /// *not* silent: `apply_edits` reports `rows_added`/`columns_added`.
    fn growth(&mut self, edits: &[Command]) -> Result<Vec<Command>, Refusal> {
        let (mut want_rows, mut want_cols) = (0u32, 0u32);
        for edit in edits {
            let (row, col) = match edit {
                Command::SetValue { row, col, .. }
                | Command::SetFormula { row, col, .. }
                | Command::ClearCell { row, col } => (*row, *col),
                _ => continue,
            };
            want_rows = want_rows.max(row + 1);
            want_cols = want_cols.max(col + 1);
        }
        self.session.settle();
        let have_rows = self.session.state().row_order().len() as u32;
        let have_cols = self.session.state().col_order().len() as u32;

        let rows = want_rows.saturating_sub(have_rows);
        let cols = want_cols.saturating_sub(have_cols);
        if rows > MAX_GROWTH_PER_CALL || cols > MAX_GROWTH_PER_CALL {
            return Err(Refusal::growth_too_large(
                rows.max(cols),
                MAX_GROWTH_PER_CALL,
            ));
        }

        let mut out = Vec::new();
        for n in have_cols..want_cols {
            out.push(Command::InsertCol { before: n });
        }
        for n in have_rows..want_rows {
            out.push(Command::InsertRow { before: n });
        }
        Ok(out)
    }

    /// Refuses a batch that cannot be applied, before anything is applied.
    ///
    /// This is what makes `apply_edits` atomic in the way that matters: the
    /// failure modes are all validation failures, so checking every edit first
    /// means a batch either runs completely or does not start.
    fn validate(&mut self, edits: &[Command]) -> Result<(), Refusal> {
        if edits.is_empty() {
            return Err(Refusal::invalid("edits must not be empty"));
        }
        if edits.len() > MAX_EDITS_PER_CALL {
            return Err(Refusal::too_many(edits.len(), MAX_EDITS_PER_CALL));
        }
        let mut scratch = Session::new(self.session.actor());
        scratch.integrate_batch(self.session.log.ops().to_vec());
        for edit in edits {
            scratch
                .apply(edit.clone())
                .map_err(|e| Refusal::command(e, "validate"))?;
        }
        Ok(())
    }

    fn version(&mut self) -> Json {
        let hash = self.state_hash();
        let generation = self.session.engine().generation();
        Json::Object(vec![
            (String::from("state_hash"), string(&hash)),
            (String::from("generation"), number(generation as f64)),
        ])
    }

    fn state_hash(&mut self) -> String {
        hex(self.session.state().state_hash().as_bytes())
    }

    fn value_json(&mut self, row: RowId, col: ColId) -> Json {
        let value = self.session.value(row, col);
        value_json(value.as_ref())
    }
}

// ------------------------------------------------------------------ values

/// A cell value as JSON. **Text goes through [`untrusted`]**, numbers and
/// booleans do not — a number cannot carry an instruction.
fn value_json(value: Option<&Value>) -> Json {
    let (tag, payload) = match value {
        None | Some(Value::Blank) => ("blank", Json::Null),
        Some(Value::Bool(b)) => ("logical", Json::Bool(*b)),
        Some(Value::Number(n)) => ("number", number(*n)),
        Some(Value::Decimal(d)) => ("number", number(d.to_f64())),
        Some(Value::Text(s)) => ("text", untrusted(s)),
        Some(Value::Error(e)) => ("error", string(e.kind.as_str())),
    };
    Json::Object(vec![
        (String::from("type"), string(tag)),
        (String::from("value"), payload),
    ])
}

fn origin_text(origin: &usk_types::Origin) -> String {
    match origin {
        usk_types::Origin::Authored => String::from("authored"),
        usk_types::Origin::Coercion { from, to } => {
            let mut out = String::from("coercion ");
            out.push_str(type_name(*from));
            out.push_str(" -> ");
            out.push_str(type_name(*to));
            out
        }
        usk_types::Origin::Arithmetic { op } => {
            let mut out = String::from("arithmetic ");
            out.push_str(match op {
                usk_types::ArithOp::Add => "+",
                usk_types::ArithOp::Sub => "-",
                usk_types::ArithOp::Mul => "*",
                usk_types::ArithOp::Div => "/",
            });
            out
        }
        usk_types::Origin::Propagated => String::from("propagated from another cell"),
    }
}

fn type_name(tag: usk_types::TypeTag) -> &'static str {
    match tag {
        usk_types::TypeTag::Blank => "blank",
        usk_types::TypeTag::Bool => "logical",
        usk_types::TypeTag::Number => "number",
        usk_types::TypeTag::Decimal => "decimal",
        usk_types::TypeTag::Text => "text",
        usk_types::TypeTag::Error => "error",
    }
}

fn count_filled(state: &State) -> usize {
    let rows = state.row_order();
    let cols = state.col_order();
    let mut filled = 0usize;
    for row in &rows {
        for col in &cols {
            if !matches!(state.cell(*row, *col), None | Some(Value::Blank)) {
                filled += 1;
            }
        }
    }
    filled
}

fn snapshot(session: &mut Session) -> Vec<((u32, u32), Value)> {
    session.settle();
    let rows = session.state().row_order();
    let cols = session.state().col_order();
    let mut out = Vec::new();
    for (r, row) in rows.iter().enumerate() {
        for (c, col) in cols.iter().enumerate() {
            if let Some(value) = session.value(*row, *col) {
                if !matches!(value, Value::Blank) {
                    out.push(((r as u32, c as u32), value));
                }
            }
        }
    }
    out
}

// ----------------------------------------------------------------- arguments

struct Ordinals {
    row0: u32,
    row1: u32,
    col0: u32,
    col1: u32,
}

impl Ordinals {
    fn from_args(args: &Json) -> Result<Ordinals, Refusal> {
        let range = args
            .get("range")
            .and_then(Json::as_str)
            .ok_or_else(|| Refusal::invalid("range required, e.g. \"A1:C10\""))?;
        let (start, end) = match range.split_once(':') {
            Some((a, b)) => (a, b),
            None => (range, range),
        };
        let a = usk_formula::parse::parse_a1(start)
            .ok_or_else(|| Refusal::invalid("range start is not an A1 reference"))?;
        let b = usk_formula::parse::parse_a1(end)
            .ok_or_else(|| Refusal::invalid("range end is not an A1 reference"))?;
        Ok(Ordinals {
            row0: a.row.min(b.row),
            row1: a.row.max(b.row),
            col0: a.col.min(b.col),
            col1: a.col.max(b.col),
        })
    }

    fn cell_from_args(args: &Json) -> Result<(u32, u32), Refusal> {
        let reference = args
            .get("cell")
            .and_then(Json::as_str)
            .ok_or_else(|| Refusal::invalid("cell required, e.g. \"B2\""))?;
        let a1 = usk_formula::parse::parse_a1(reference)
            .ok_or_else(|| Refusal::invalid("cell is not an A1 reference"))?;
        Ok((a1.row, a1.col))
    }

    fn cell_count(&self) -> u64 {
        (self.row1 - self.row0 + 1) as u64 * (self.col1 - self.col0 + 1) as u64
    }
}

fn parse_edits(args: &Json) -> Result<Vec<Command>, Refusal> {
    let items = args
        .get("edits")
        .and_then(Json::as_array)
        .ok_or_else(|| Refusal::invalid("edits array required"))?;
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        let reference = item
            .get("cell")
            .and_then(Json::as_str)
            .ok_or_else(|| Refusal::invalid("each edit needs a cell"))?;
        let a1 = usk_formula::parse::parse_a1(reference)
            .ok_or_else(|| Refusal::invalid("edit cell is not an A1 reference"))?;

        let command = if let Some(formula) = item.get("formula").and_then(Json::as_str) {
            Command::SetFormula {
                row: a1.row,
                col: a1.col,
                source: formula.to_string(),
            }
        } else if let Some(value) = item.get("value") {
            match value {
                Json::Null => Command::ClearCell {
                    row: a1.row,
                    col: a1.col,
                },
                Json::Bool(b) => Command::SetValue {
                    row: a1.row,
                    col: a1.col,
                    value: Value::Bool(*b),
                },
                Json::Number(_) => Command::SetValue {
                    row: a1.row,
                    col: a1.col,
                    value: Value::Number(value.as_f64().unwrap_or(0.0)),
                },
                Json::String(s) => Command::SetValue {
                    row: a1.row,
                    col: a1.col,
                    value: Value::Text(s.clone()),
                },
                _ => return Err(Refusal::invalid("edit value must be a scalar or null")),
            }
        } else {
            return Err(Refusal::invalid("each edit needs a value or a formula"));
        };
        out.push(command);
    }
    Ok(out)
}

// ------------------------------------------------------------------- errors

/// A **tool** declining. Distinct from [`RpcError`]: the call succeeded and the
/// tool said no, which is information an agent can act on.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Refusal {
    pub code: &'static str,
    pub message: String,
}

impl Refusal {
    fn new(code: &'static str, message: impl Into<String>) -> Refusal {
        Refusal {
            code,
            message: message.into(),
        }
    }

    fn invalid(message: &str) -> Refusal {
        Refusal::new("invalid_arguments", message)
    }

    fn out_of_range(what: &str) -> Refusal {
        let mut message = String::from("no such ");
        message.push_str(what);
        message.push_str(" in the current view");
        Refusal::new("out_of_range", message)
    }

    fn too_many(given: usize, cap: usize) -> Refusal {
        let mut message = String::from("too many edits: ");
        message.push_str(&given.to_string());
        message.push_str(" (cap ");
        message.push_str(&cap.to_string());
        message.push(')');
        Refusal::new("too_many_edits", message)
    }

    fn command(err: usk_reduce::CommandError, during: &str) -> Refusal {
        let mut message = String::from(match err {
            usk_reduce::CommandError::OutOfRange => "an edit names a cell outside the grid",
            usk_reduce::CommandError::UnboundReference => "a formula reference could not be bound",
        });
        message.push_str(" (during ");
        message.push_str(during);
        message.push(')');
        Refusal::new("edit_refused", message)
    }

    fn version_mismatch(expected: &str, actual: &str) -> Refusal {
        let mut message = String::from("the workbook has changed since you read it: expected ");
        message.push_str(expected);
        message.push_str(", now ");
        message.push_str(actual);
        Refusal::new("version_mismatch", message)
    }

    fn growth_too_large(needed: u32, cap: u32) -> Refusal {
        let mut message = String::from("that edit would extend the sheet by ");
        message.push_str(&needed.to_string());
        message.push_str(" rows or columns in one call (cap ");
        message.push_str(&cap.to_string());
        message.push_str("); insert them deliberately instead");
        Refusal::new("growth_too_large", message)
    }

    fn preview_required(threshold: u64) -> Refusal {
        let mut message = String::from(
            "this edit changes more than the blast-radius threshold; call preview_edits first and pass its preview_hash (threshold ",
        );
        message.push_str(&threshold.to_string());
        message.push_str(" cells)");
        Refusal::new("preview_required", message)
    }

    fn preview_mismatch(given: &str, expected: &str) -> Refusal {
        let mut message = String::from("preview_hash does not match the pending preview: got ");
        message.push_str(given);
        message.push_str(", expected ");
        message.push_str(expected);
        Refusal::new("preview_mismatch", message)
    }

    fn to_json(&self) -> Json {
        Json::Object(vec![
            (String::from("refused"), Json::Bool(true)),
            (String::from("code"), string(self.code)),
            (String::from("message"), string(&self.message)),
        ])
    }
}

/// A JSON-RPC transport error: the request itself was wrong.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct RpcError {
    pub code: i32,
    pub message: String,
}

impl RpcError {
    fn method_not_found(method: &str) -> RpcError {
        let mut message = String::from("unknown method: ");
        message.push_str(method);
        RpcError {
            code: -32601,
            message,
        }
    }

    fn unknown_tool(name: &str) -> RpcError {
        let mut message = String::from("unknown tool: ");
        message.push_str(name);
        RpcError {
            code: -32602,
            message,
        }
    }

    fn invalid_params(message: &str) -> RpcError {
        RpcError {
            code: -32602,
            message: String::from(message),
        }
    }

    fn to_json(&self) -> Json {
        Json::Object(vec![
            (String::from("code"), number(self.code as f64)),
            (String::from("message"), string(&self.message)),
        ])
    }
}

/// MCP wraps a tool's answer in content blocks, with `isError` distinguishing a
/// refusal from an answer.
fn tool_result(payload: Json, is_error: bool) -> Json {
    Json::Object(vec![
        (
            String::from("content"),
            Json::Array(vec![Json::Object(vec![
                (String::from("type"), string("text")),
                (String::from("text"), string(payload.to_json_string())),
            ])]),
        ),
        (String::from("structuredContent"), payload),
        (String::from("isError"), Json::Bool(is_error)),
    ])
}

// ------------------------------------------------------------------ naming

/// 0-based column ordinal → `A`, `Z`, `AA`.
pub fn column_name(mut col: u32) -> String {
    let mut out = String::new();
    loop {
        out.insert(0, (b'A' + (col % 26) as u8) as char);
        if col < 26 {
            return out;
        }
        col = col / 26 - 1;
    }
}

pub fn a1(row: u32, col: u32) -> String {
    let mut out = column_name(col);
    out.push_str(&(row + 1).to_string());
    out
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(DIGITS[(byte >> 4) as usize] as char);
        out.push(DIGITS[(byte & 0x0F) as usize] as char);
    }
    out
}
