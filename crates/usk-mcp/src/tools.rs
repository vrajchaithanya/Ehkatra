//! The tool catalogue and its JSON Schemas (docs/21: *versioned JSON-Schema
//! I/O*).
//!
//! Schemas are written out as data rather than derived, because they are a
//! **contract with a client we do not control**. A derived schema changes when
//! a struct is refactored; a written one changes when someone decides it
//! should, which is the property a published interface needs.
//!
//! `tools_are_the_ones_bootstrap_row_14_lists` pins the catalogue against
//! BOOTSTRAP so a tool cannot quietly disappear.

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use usk_json::{string, Json};

/// Every tool this server exposes, in BOOTSTRAP row 14's order.
pub const TOOLS: &[&str] = &[
    "describe_workbook",
    "describe_sheet",
    "read_range",
    "explain_cell",
    "preview_edits",
    "apply_edits",
    "undo",
];

fn obj(fields: Vec<(&str, Json)>) -> Json {
    Json::Object(
        fields
            .into_iter()
            .map(|(k, v)| (String::from(k), v))
            .collect(),
    )
}

fn strings(items: &[&str]) -> Json {
    Json::Array(items.iter().map(|s| string(*s)).collect())
}

fn prop(kind: &str, description: &str) -> Json {
    obj(vec![
        ("type", string(kind)),
        ("description", string(description)),
    ])
}

fn schema(properties: Vec<(&str, Json)>, required: &[&str]) -> Json {
    obj(vec![
        ("type", string("object")),
        ("properties", obj(properties)),
        ("required", strings(required)),
        // Closed by construction: an argument we do not know is a client
        // disagreeing with us about the contract, and silently ignoring it
        // means the client believes something happened that did not.
        ("additionalProperties", Json::Bool(false)),
    ])
}

/// The `edits` array shared by `preview_edits` and `apply_edits`. One
/// definition, so the two tools cannot drift apart — an agent that previews
/// one shape and applies another has previewed nothing.
fn edits_property() -> Json {
    obj(vec![
        ("type", string("array")),
        (
            "description",
            string("Cell edits. Each names a cell and either a literal value or a formula."),
        ),
        (
            "items",
            obj(vec![
                ("type", string("object")),
                (
                    "properties",
                    obj(vec![
                        ("cell", prop("string", "A1-style reference, e.g. \"B2\"")),
                        (
                            "value",
                            obj(vec![(
                                "description",
                                string(
                                    "A literal number, string or boolean. null clears the cell.",
                                ),
                            )]),
                        ),
                        (
                            "formula",
                            prop(
                                "string",
                                "A formula, with or without the leading '='. Mutually exclusive with value.",
                            ),
                        ),
                    ]),
                ),
                ("required", strings(&["cell"])),
            ]),
        ),
    ])
}

fn tool(name: &str, description: &str, input: Json) -> Json {
    obj(vec![
        ("name", string(name)),
        ("description", string(description)),
        ("inputSchema", input),
    ])
}

pub fn list() -> Json {
    obj(vec![(
        "tools",
        Json::Array(vec![
            tool(
                "describe_workbook",
                "Shape of the workbook: sheets, dimensions, how many cells are filled and how \
                 many hold formulas, plus the version to quote back in apply_edits. Returns no \
                 cell contents.",
                schema(vec![], &[]),
            ),
            tool(
                "describe_sheet",
                "Per-column type statistics and up to five sample rows. Bounded at any sheet \
                 size: ask this before read_range. Cell-derived text is labelled untrusted.",
                schema(vec![], &[]),
            ),
            tool(
                "read_range",
                "Cell values for an A1 range. Capped; the response states whether it was \
                 truncated. This is the escape hatch, not the way to explore a sheet.",
                schema(
                    vec![(
                        "range",
                        prop("string", "An A1 range such as \"A1:D20\", or a single cell."),
                    )],
                    &["range"],
                ),
            ),
            tool(
                "explain_cell",
                "What one cell holds, what it computed, and — when it is an error — where the \
                 error came from.",
                schema(
                    vec![("cell", prop("string", "An A1 reference such as \"C7\"."))],
                    &["cell"],
                ),
            ),
            tool(
                "preview_edits",
                "Simulates edits without applying them. Returns an impact report (cells changed, \
                 downstream cells affected, errors introduced) and a preview_hash. Required \
                 before a large apply_edits.",
                schema(vec![("edits", edits_property())], &["edits"]),
            ),
            tool(
                "apply_edits",
                "Applies edits as one labelled, reversible batch. Pass expected_version to refuse \
                 if the workbook has changed since you read it. Above the blast-radius threshold \
                 a matching preview_hash is required.",
                schema(
                    vec![
                        ("edits", edits_property()),
                        (
                            "label",
                            prop("string", "A short description of the change, shown to the user."),
                        ),
                        (
                            "expected_version",
                            prop(
                                "string",
                                "The state_hash you last read. The call is refused if it no longer matches.",
                            ),
                        ),
                        (
                            "preview_hash",
                            prop("string", "The preview_hash returned by preview_edits for these edits."),
                        ),
                    ],
                    &["edits"],
                ),
            ),
            tool(
                "undo",
                "Reverses the most recent apply_edits batch as a unit. Reports how many cells \
                 could not be restored because another author now owns them.",
                schema(vec![], &[]),
            ),
        ]),
    )])
}
