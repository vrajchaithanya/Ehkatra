//! The streaming CSV parser (docs/24: *"CSV/TSV in/out (streaming ...)"*).
//!
//! A push state machine: the caller supplies bytes in whatever chunks it has
//! and drains completed records. Nothing here needs the whole document, and a
//! record boundary that falls in the middle of a chunk — or in the middle of a
//! quoted field containing a newline — is the ordinary case rather than an edge
//! one.
//!
//! Grammar is RFC 4180 plus the two deviations every real file has: bare `LF`
//! line endings alongside `CRLF`, and a UTF-8 BOM.

use alloc::string::String;
use alloc::vec::Vec;

use crate::limits::{MAX_FIELDS, MAX_FIELD_BYTES};
use crate::{CsvError, Dialect, Record};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum State {
    /// Between fields — the next byte decides whether this one is quoted.
    FieldStart,
    /// Inside an unquoted field.
    Bare,
    /// Inside a quoted field.
    Quoted,
    /// Just consumed a quote inside a quoted field: a second one is an escaped
    /// quote, anything else ends the field.
    QuoteInQuoted,
}

/// Incremental CSV parser.
pub struct CsvParser {
    dialect: Dialect,
    state: State,
    field: Vec<u8>,
    fields: Vec<String>,
    line: usize,
    record_line: usize,
    /// True once anything at all has been seen on the current record, so a
    /// trailing newline does not manufacture a phantom empty record.
    record_started: bool,
    /// The first up-to-three bytes, held back until there are enough of them to
    /// tell a BOM from data.
    ///
    /// The obvious implementation strips the BOM from the front of the first
    /// chunk, and it is wrong the moment a chunk boundary lands *inside* the
    /// three-byte sequence — the strip silently fails and `\u{FEFF}` becomes
    /// part of the first header name. The fuzz corpus found exactly that on its
    /// first run, which is the argument for asserting chunk independence rather
    /// than assuming it.
    bom_probe: Vec<u8>,
    /// True once the probe has been resolved, so it is never re-entered.
    bom_settled: bool,
    saw_cr: bool,
    finished: bool,
}

const BOM: [u8; 3] = [0xEF, 0xBB, 0xBF];

impl CsvParser {
    pub fn new(dialect: Dialect) -> CsvParser {
        CsvParser {
            dialect,
            state: State::FieldStart,
            field: Vec::new(),
            fields: Vec::new(),
            line: 1,
            record_line: 1,
            record_started: false,
            bom_probe: Vec::with_capacity(BOM.len()),
            bom_settled: false,
            saw_cr: false,
            finished: false,
        }
    }

    pub fn dialect(&self) -> Dialect {
        self.dialect
    }

    /// Feeds a chunk, appending every record it completes to `out`.
    ///
    /// Chunk boundaries are invisible to the result: the same bytes split any
    /// way produce the same records, which is the property
    /// `chunking_never_changes_the_records` proves over every split point.
    pub fn push(&mut self, chunk: &[u8], out: &mut Vec<Record>) -> Result<(), CsvError> {
        let mut bytes = chunk;

        // A UTF-8 BOM is metadata, not data. Left in place it becomes part of
        // the first header name, and every later column lookup fails for a
        // reason invisible in any diff. Deciding needs three bytes, and they do
        // not have to arrive together.
        if !self.bom_settled {
            let wanted = BOM.len() - self.bom_probe.len();
            let taken = wanted.min(bytes.len());
            self.bom_probe.extend_from_slice(&bytes[..taken]);
            bytes = &bytes[taken..];
            if self.bom_probe.len() == BOM.len() {
                self.flush_bom_probe(out)?;
            }
        }

        for &byte in bytes {
            self.feed(byte, out)?;
        }
        Ok(())
    }

    /// Releases the held-back prefix: dropped if it is the BOM, replayed as
    /// ordinary data if it is not.
    fn flush_bom_probe(&mut self, out: &mut Vec<Record>) -> Result<(), CsvError> {
        // Marked spent before replaying, so `finish` cannot release it twice
        // and a short probe cannot re-enter the probing branch.
        self.bom_settled = true;
        let probe = core::mem::take(&mut self.bom_probe);
        if probe == BOM {
            return Ok(());
        }
        for byte in probe {
            self.feed(byte, out)?;
        }
        Ok(())
    }

    fn feed(&mut self, byte: u8, out: &mut Vec<Record>) -> Result<(), CsvError> {
        // CRLF is one line ending. A lone CR inside a field stays data — some
        // exporters emit it and it is not ours to drop.
        if self.saw_cr {
            self.saw_cr = false;
            if byte == b'\n' {
                return Ok(());
            }
        }
        self.step(byte, out)
    }

    fn step(&mut self, byte: u8, out: &mut Vec<Record>) -> Result<(), CsvError> {
        let delimiter = self.dialect.delimiter;
        let quote = self.dialect.quote;
        match self.state {
            State::FieldStart => {
                self.record_started = true;
                if byte == quote {
                    self.state = State::Quoted;
                } else if byte == delimiter {
                    self.end_field()?;
                } else if byte == b'\n' || byte == b'\r' {
                    self.saw_cr = byte == b'\r';
                    self.end_record(out)?;
                } else {
                    self.field.push(byte);
                    self.state = State::Bare;
                }
            }
            State::Bare => {
                if byte == delimiter {
                    self.end_field()?;
                } else if byte == b'\n' || byte == b'\r' {
                    self.saw_cr = byte == b'\r';
                    self.end_record(out)?;
                } else {
                    self.push_byte(byte)?;
                }
            }
            State::Quoted => {
                if byte == quote {
                    self.state = State::QuoteInQuoted;
                } else {
                    if byte == b'\n' {
                        // A newline inside quotes belongs to the field, but the
                        // line counter still has to advance or every later
                        // report points at the wrong line.
                        self.line += 1;
                    }
                    self.push_byte(byte)?;
                }
            }
            State::QuoteInQuoted => {
                if byte == quote {
                    self.push_byte(quote)?;
                    self.state = State::Quoted;
                } else if byte == delimiter {
                    self.end_field()?;
                } else if byte == b'\n' || byte == b'\r' {
                    self.saw_cr = byte == b'\r';
                    self.end_record(out)?;
                } else {
                    // Text after a closing quote (`"a"b`). Excel keeps it, so
                    // this keeps it: refusing here would reject files that open
                    // fine in the program we are compatible with.
                    self.push_byte(byte)?;
                    self.state = State::Bare;
                }
            }
        }
        Ok(())
    }

    fn push_byte(&mut self, byte: u8) -> Result<(), CsvError> {
        if self.field.len() >= MAX_FIELD_BYTES {
            return Err(CsvError::FieldTooLong {
                line: self.record_line,
                bytes: self.field.len() + 1,
            });
        }
        self.field.push(byte);
        Ok(())
    }

    fn end_field(&mut self) -> Result<(), CsvError> {
        if self.fields.len() >= MAX_FIELDS {
            return Err(CsvError::TooManyFields {
                line: self.record_line,
            });
        }
        let bytes = core::mem::take(&mut self.field);
        let text = String::from_utf8(bytes).map_err(|_| CsvError::NotUtf8 {
            line: self.record_line,
        })?;
        self.fields.push(text);
        self.state = State::FieldStart;
        Ok(())
    }

    fn end_record(&mut self, out: &mut Vec<Record>) -> Result<(), CsvError> {
        self.end_field()?;
        let fields = core::mem::take(&mut self.fields);
        out.push(Record {
            fields,
            line: self.record_line,
        });
        self.line += 1;
        self.record_line = self.line;
        self.record_started = false;
        Ok(())
    }

    /// Ends the document, flushing any final record that had no trailing
    /// newline. Idempotent.
    pub fn finish(&mut self, out: &mut Vec<Record>) -> Result<(), CsvError> {
        if self.finished {
            return Ok(());
        }
        self.finished = true;
        // A document shorter than a BOM still has to release its bytes.
        if !self.bom_settled {
            self.flush_bom_probe(out)?;
        }
        if self.state == State::Quoted {
            return Err(CsvError::UnterminatedQuote {
                line: self.record_line,
            });
        }
        if self.record_started || !self.field.is_empty() || !self.fields.is_empty() {
            self.end_record(out)?;
        }
        Ok(())
    }
}

/// Parses a whole document at once. A convenience over [`CsvParser`] for
/// callers that already hold the bytes — the streaming path is the real one.
pub fn parse_all(bytes: &[u8], dialect: Dialect) -> Result<Vec<Record>, CsvError> {
    let mut parser = CsvParser::new(dialect);
    let mut out = Vec::new();
    parser.push(bytes, &mut out)?;
    parser.finish(&mut out)?;
    Ok(out)
}
