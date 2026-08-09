//! ehkatra-io — the host side of import/export (docs/24, BOOTSTRAP row 12).
//!
//! The division of labour is the whole design:
//! * **`usk-csv`** holds every rule — grammar, inference, injection — and has
//!   no I/O, so all of it is provable against hostile bytes without a
//!   filesystem.
//! * **`ehkatra-parse`** is the untrusted process that runs those rules on a
//!   real document.
//! * **This crate** spawns that process, confines it, caps it, and revalidates
//!   what comes back.
//!
//! docs/24 says the sandbox rule has *"no exceptions"*, so there is no
//! in-process import function here to reach for on a tired afternoon. The only
//! way to import a document is [`import_csv`], and it spawns.

pub mod ir;
pub mod sandbox;
pub mod workbook_ir;

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Instant;

use ir::{Ir, IrError};
use sandbox::Sandbox;

/// Why an import did not happen. Distinct from a *parse* failure, which is a
/// well-formed answer about a malformed file and arrives as `Ir::Failed`.
#[derive(Debug)]
pub enum ImportError {
    /// The parser binary could not be found or started.
    Spawn(std::io::Error),
    /// Confinement could not be applied. **Fatal by design**: a sandbox that
    /// silently degrades to no sandbox is what docs/24's "no exceptions" is
    /// written against.
    NotConfined(std::io::Error),
    Io(std::io::Error),
    /// The child exceeded [`sandbox::MAX_WALL`] and was killed with its job.
    Timeout,
    /// The child produced more than [`sandbox::MAX_IR_BYTES`].
    OutputTooLarge,
    /// The child exited without producing usable IR.
    Died {
        status: Option<i32>,
    },
    /// The child's output failed revalidation — it broke a bound it was itself
    /// supposed to enforce, which means it is compromised or broken.
    BadIr(IrError),
}

impl std::fmt::Display for ImportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ImportError::Spawn(e) => write!(f, "could not start the sandboxed parser: {e}"),
            ImportError::NotConfined(e) => write!(f, "refusing to parse unconfined: {e}"),
            ImportError::Io(e) => write!(f, "io: {e}"),
            ImportError::Timeout => write!(f, "the parser exceeded its wall-clock cap"),
            ImportError::OutputTooLarge => write!(f, "the parser produced more IR than allowed"),
            ImportError::Died { status } => write!(f, "the parser died (status {status:?})"),
            ImportError::BadIr(e) => write!(f, "the parser's output failed revalidation: {e:?}"),
        }
    }
}

impl std::error::Error for ImportError {}

/// A completed import: what the parser said, and how long it was allowed to
/// take saying it.
pub struct Imported {
    pub ir: Ir,
    pub elapsed_ms: u128,
    /// Whether OS-level resource limits were in force. False on platforms
    /// where the job-object equivalents are not yet ported — reported rather
    /// than assumed, so a caller handling genuinely hostile files can refuse.
    pub confined: bool,
}

/// Imports a CSV document **through the sandbox**. There is no other way.
pub fn import_csv(path: &Path, header: bool) -> Result<Imported, ImportError> {
    let bytes = std::fs::read(path).map_err(ImportError::Io)?;
    import_csv_bytes(&bytes, header)
}

/// Imports an XLSX workbook **through the sandbox**, like everything else.
///
/// docs/24's sandbox rule says "no exceptions", and XLSX is the format that
/// most needs it: a ZIP of compressed XML is three parsers deep before any
/// spreadsheet semantics appear.
pub fn import_xlsx(path: &Path) -> Result<Imported, ImportError> {
    let bytes = std::fs::read(path).map_err(ImportError::Io)?;
    import_xlsx_bytes(&bytes)
}

pub fn import_xlsx_bytes(bytes: &[u8]) -> Result<Imported, ImportError> {
    run_parser(bytes, &["xlsx"])
}

/// Imports bytes the caller already holds, still through the sandbox.
///
/// The document travels on **stdin**, not as a path: the child then has no
/// reason to open a file and no path to be confused about. The host chose the
/// document; the child only ever sees bytes.
pub fn import_csv_bytes(bytes: &[u8], header: bool) -> Result<Imported, ImportError> {
    if header {
        run_parser(bytes, &["csv"])
    } else {
        run_parser(bytes, &["csv", "--no-header"])
    }
}

/// Spawns, confines, feeds, reads and revalidates. One path for every format,
/// so a new format cannot arrive with a slightly different — or absent —
/// sandbox.
fn run_parser(bytes: &[u8], args: &[&str]) -> Result<Imported, ImportError> {
    let started = Instant::now();
    let mut command = Command::new(parser_binary());
    command.args(args);
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(ImportError::Spawn)?;

    // Confine before writing a single byte of the document. Ordering is the
    // point: a parser that receives its input before it is confined has already
    // had its chance.
    let sandbox = match Sandbox::confine(&child) {
        Ok(sandbox) => sandbox,
        Err(err) => {
            let _ = child.kill();
            return Err(ImportError::NotConfined(err));
        }
    };

    let input = bytes.to_vec();
    let mut stdin = child
        .stdin
        .take()
        .ok_or(ImportError::Died { status: None })?;
    // The write happens on another thread because a child that stops reading
    // while its stdout pipe fills will deadlock a single-threaded host — the
    // classic pipe deadlock, and a denial of service a hostile file could
    // trigger on purpose.
    let writer = std::thread::spawn(move || {
        let _ = stdin.write_all(&input);
        drop(stdin);
    });

    let mut stdout = child
        .stdout
        .take()
        .ok_or(ImportError::Died { status: None })?;
    let mut out = Vec::new();
    let mut buffer = [0u8; 64 << 10];
    let mut overflowed = false;
    loop {
        if started.elapsed() > sandbox::MAX_WALL {
            sandbox.terminate();
            let _ = writer.join();
            return Err(ImportError::Timeout);
        }
        match stdout.read(&mut buffer) {
            Ok(0) => break,
            Ok(n) => {
                if out.len() + n > sandbox::MAX_IR_BYTES {
                    overflowed = true;
                    sandbox.terminate();
                    break;
                }
                out.extend_from_slice(&buffer[..n]);
            }
            Err(err) => {
                sandbox.terminate();
                let _ = writer.join();
                return Err(ImportError::Io(err));
            }
        }
    }
    let _ = writer.join();

    if overflowed {
        return Err(ImportError::OutputTooLarge);
    }

    let status = child.wait().map_err(ImportError::Io)?;
    if !status.success() && out.is_empty() {
        return Err(ImportError::Died {
            status: status.code(),
        });
    }

    let ir = ir::decode(&out).map_err(ImportError::BadIr)?;
    Ok(Imported {
        ir,
        elapsed_ms: started.elapsed().as_millis(),
        confined: sandbox.is_confined(),
    })
}

/// The parser binary, resolved next to the current executable.
///
/// Deliberately **not** a `PATH` lookup: the sandbox is worth nothing if an
/// attacker who can drop a file earlier in `PATH` gets to choose what runs
/// inside it.
fn parser_binary() -> PathBuf {
    let name = if cfg!(windows) {
        "ehkatra-parse.exe"
    } else {
        "ehkatra-parse"
    };
    let Some(dir) = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf))
    else {
        return PathBuf::from(name);
    };
    let beside = dir.join(name);
    if beside.exists() {
        return beside;
    }
    // A test binary lives one level deeper (`target/debug/deps/`) than the
    // binaries it exercises. Looking one directory up covers that without
    // widening the search to `PATH`, which is the thing that must not happen:
    // a sandbox is worth nothing if an attacker who can drop a file earlier in
    // `PATH` chooses what runs inside it.
    let up = dir.parent().map(|parent| parent.join(name));
    match up {
        Some(path) if path.exists() => path,
        _ => beside,
    }
}
