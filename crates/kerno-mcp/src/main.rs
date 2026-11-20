//! `kerno-mcp --db <dir>`: see the crate README.

use std::path::PathBuf;
use std::process::ExitCode;

use kerno_mcp::{Memory, serve};

const USAGE: &str = "usage: kerno-mcp --db <dir>   (or set KERNO_MCP_DB=<dir>)\n\
    Serves MCP over stdin/stdout; the database in <dir> is created if it does not exist.";

/// The database directory: `--db <dir>` (or `--db=<dir>`), else `KERNO_MCP_DB`.
/// `Ok(None)` means help was asked for.
fn database(args: impl Iterator<Item = String>, env: Option<String>) -> Result<Option<PathBuf>, String> {
    let mut dir = None;
    let mut args = args;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => return Ok(None),
            "--db" => dir = Some(args.next().ok_or("--db needs a directory")?),
            other => match other.strip_prefix("--db=") {
                Some(d) => dir = Some(d.to_owned()),
                None => return Err(format!("unknown argument {other:?}")),
            },
        }
    }
    match dir.or(env).filter(|d| !d.is_empty()) {
        Some(d) => Ok(Some(PathBuf::from(d))),
        None => Err("no database: pass --db <dir> or set KERNO_MCP_DB".into()),
    }
}

fn run() -> Result<(), String> {
    let Some(dir) = database(std::env::args().skip(1), std::env::var("KERNO_MCP_DB").ok())? else {
        println!("{USAGE}");
        return Ok(());
    };
    let memory = Memory::open(&dir)?;
    let served = serve(&memory, std::io::stdin().lock(), std::io::stdout().lock());
    // Close even when the client went away mid-write: that is how a session ends.
    let closed = memory.close();
    served.map_err(|e| format!("i/o error: {e}"))?;
    closed
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("kerno-mcp: {e}\n{USAGE}");
            ExitCode::FAILURE
        }
    }
}
