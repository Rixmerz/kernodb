//! `kerno <dir> <command> ...`: administration of a Kerno database directory.
//!
//! The commands are in [`cmd`], their text and JSON in [`render`]; this file
//! parses the line, prints, and maps every failure to an exit code:
//! 0 success, 1 failure (including a `verify` that found something), 2 usage.

mod cmd;
mod render;

use std::collections::BTreeMap;
use std::io::Write;
use std::path::Path;
use std::process::ExitCode;

use serde_json::Value;

/// Why a command did not run to its end.
#[derive(Debug)]
pub enum CliError {
    /// The command line is wrong: exit 2.
    Usage(String),
    /// The line is right and the work failed: exit 1.
    Failure(String),
}

impl From<kerno_units::UnitError> for CliError {
    fn from(e: kerno_units::UnitError) -> Self {
        CliError::Failure(e.to_string())
    }
}

impl From<kerno_core::Error> for CliError {
    fn from(e: kerno_core::Error) -> Self {
        CliError::Failure(e.to_string())
    }
}

impl From<std::io::Error> for CliError {
    fn from(e: std::io::Error) -> Self {
        CliError::Failure(e.to_string())
    }
}

/// What a command answers: the same facts as text and as JSON.
pub struct Output {
    pub text: String,
    pub json: Value,
    /// A `verify` that found something: printed like a success, exit 1.
    pub failed: bool,
}

#[derive(Clone, Copy)]
enum Flag {
    /// `--name`
    Switch(&'static str),
    /// `--name <value>`
    Value(&'static str),
}

impl Flag {
    fn name(self) -> &'static str {
        match self {
            Flag::Switch(n) | Flag::Value(n) => n,
        }
    }
}

struct Spec {
    name: &'static str,
    /// Arguments after the command, for the usage line.
    usage: &'static str,
    help: &'static str,
    flags: &'static [Flag],
    /// How many arguments after the command: a count, or a range for an optional one.
    positionals: std::ops::RangeInclusive<usize>,
}

const JSON: Flag = Flag::Switch("--json");

const SPECS: &[Spec] = &[
    Spec {
        name: "verify",
        usage: "[--deep] [--json]",
        help: "Check the database and print every finding; exit 1 if there is any.\n\
               Runs the engine's file check (with --deep it reads every block of every table)\n\
               and the units layer's verify_units. Never writes.",
        flags: &[Flag::Switch("--deep"), JSON],
        positionals: 0..=0,
    },
    Spec {
        name: "stats",
        usage: "[--json]",
        help: "Units by type, relations by type, index state, layer and format version, and the\n\
               files on disk with their sizes. Reads every unit: slow on a large database.",
        flags: &[JSON],
        positionals: 0..=0,
    },
    Spec {
        name: "get",
        usage: "<unit-id> [--json]",
        help: "One unit with its current revision and payload (text if valid UTF-8, else hex\n\
               and length). <unit-id> is the 32-digit hex id or a public id (k1_...).",
        flags: &[JSON],
        positionals: 1..=1,
    },
    Spec {
        name: "history",
        usage: "<unit-id> [--json]",
        help: "The retained revisions of a unit, oldest first.",
        flags: &[JSON],
        positionals: 1..=1,
    },
    Spec {
        name: "relations",
        usage: "<unit-id> [--dir in|out] [--type T] [--json]",
        help: "The relations of a unit through the planner. Without --dir, both directions.",
        flags: &[Flag::Value("--dir"), Flag::Value("--type"), JSON],
        positionals: 1..=1,
    },
    Spec {
        name: "query",
        usage: "(<query> | --type T [--limit N]) [--explain] [--json]",
        help: "Run a query. <query> is the query language of docs/query-language.md: filters, ORDER BY,\n\
               LIMIT and joins across unit types through typed relations, over the units of types\n\
               that have a schema, e.g. kerno <dir> query 'FIND user AS u JOIN team AS t VIA member_of\n\
               WHERE u.age >= 30 ORDER BY u.age DESC LIMIT 10'. A malformed query is a usage error.\n\
               --type T lists the units of one type through the planner, in unit id order (--limit\n\
               cuts the list; the language has its own LIMIT). --explain also prints the plan: every\n\
               candidate access path with its cost, the one chosen and why.",
        flags: &[Flag::Value("--type"), Flag::Value("--limit"), Flag::Switch("--explain"), JSON],
        positionals: 0..=1,
    },
    Spec {
        name: "export",
        usage: "<file>",
        help: "Write the database to <file> in the export format (docs/file-format/export-v1.md).\n\
               The format is frozen at format_version 1: a later Kerno reads this file, and the same\n\
               logical state gives the same bytes. Refuses to overwrite <file>.",
        flags: &[],
        positionals: 1..=1,
    },
    Spec {
        name: "import",
        usage: "<file> <new-dir>",
        help: "Create a database in <new-dir> from an export file; <new-dir> must not exist or be\n\
               empty. Written as `kerno import <file> <new-dir>`; `kerno <dir> import ...` is\n\
               accepted and <dir> is ignored. Reads the frozen export format v1 (docs/file-format/export-v1.md).",
        flags: &[],
        positionals: 2..=2,
    },
    Spec {
        name: "backup",
        usage: "<url> [--json]",
        help: "Copy the whole database to <url> (file:///dir or s3://bucket/prefix) as a base backup: every\n\
               key at one commit, the starting point of ship and restore. Opens the database read-only,\n\
               so it needs the database closed; a running kerno-server takes its own (--backup).",
        flags: &[JSON],
        positionals: 1..=1,
    },
    Spec {
        name: "ship",
        usage: "<url> [--json]",
        help: "Upload the commits made since the last base backup or shipment, read from the\n\
               write-ahead logs. It reads files only, so it also works while a server has the\n\
               database open. Needs a base backup in <url> (see backup); if the engine already deleted\n\
               a log that was not shipped, it fails and a new base backup is needed.",
        flags: &[JSON],
        positionals: 1..=1,
    },
    Spec {
        name: "restore",
        usage: "<url> [--to-seq N | --to-time MS] [--json]",
        help: "Rebuild the database from <url> into <dir>, which must not exist or be empty. Without an\n\
               option it goes as far as the backup does; --to-seq N stops at commit N exactly (an error\n\
               if the backup ends sooner); --to-time MS (milliseconds since the Unix epoch) stops at what\n\
               had been shipped by then. Built beside <dir> and moved in when whole.",
        flags: &[Flag::Value("--to-seq"), Flag::Value("--to-time"), JSON],
        positionals: 1..=1,
    },
    Spec {
        name: "backups",
        usage: "<url> [--json]",
        help: "List the base backups and shipped segments in <url> and how far a restore can go.\n\
               <dir> is ignored (the listing reads <url> only), as it is for import.",
        flags: &[JSON],
        positionals: 1..=1,
    },
    Spec {
        name: "reclaim",
        usage: "",
        help: "Compact the live files so they stop holding what was deleted logically.\n\
               This is not a secure erase: a file an open snapshot retains stays, and the engine\n\
               does not overwrite freed blocks, so nothing is promised about the storage medium\n\
               (ADR-0009). The only command that writes to the database.",
        flags: &[],
        positionals: 0..=0,
    },
];

fn help() -> String {
    let mut s = String::from(
        "kerno: administration of a Kerno database directory\n\n\
         usage: kerno <dir> <command> [args]\n\
         \x20      kerno import <file> <new-dir>\n\
         \x20      kerno <dir> <command> --help\n\n\
         Read commands open the database read-only and never write (I-18); `--json` prints JSON.\n\
         Exit status: 0 success, 1 failure or findings, 2 usage error.\n\n\
         commands:\n",
    );
    for spec in SPECS {
        s.push_str(&command_help(spec));
        s.push('\n');
    }
    s
}

fn command_help(spec: &Spec) -> String {
    let line = if spec.name == "import" {
        format!("  kerno import {}", spec.usage)
    } else {
        format!("  kerno <dir> {} {}", spec.name, spec.usage)
    };
    let body: String = spec.help.lines().map(|l| format!("      {l}\n")).collect();
    format!("{}\n{body}", line.trim_end())
}

pub struct Opts {
    pub pos: Vec<String>,
    set: BTreeMap<&'static str, Option<String>>,
}

impl Opts {
    pub fn has(&self, flag: &str) -> bool {
        self.set.contains_key(flag)
    }

    pub fn value(&self, flag: &str) -> Option<&str> {
        self.set.get(flag).and_then(|v| v.as_deref())
    }
}

fn parse(args: &[String], spec: &Spec) -> Result<Opts, CliError> {
    let mut opts = Opts { pos: Vec::new(), set: BTreeMap::new() };
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        if !arg.starts_with("--") || arg == "--" {
            opts.pos.push(arg.clone());
            continue;
        }
        let Some(flag) = spec.flags.iter().find(|f| f.name() == arg) else {
            return Err(CliError::Usage(format!("{}: unknown option {arg}", spec.name)));
        };
        let value = match flag {
            Flag::Switch(_) => None,
            Flag::Value(_) => {
                Some(it.next().ok_or_else(|| CliError::Usage(format!("{}: {arg} needs a value", spec.name)))?.clone())
            }
        };
        if opts.set.insert(flag.name(), value).is_some() {
            return Err(CliError::Usage(format!("{}: {arg} given twice", spec.name)));
        }
    }
    if !spec.positionals.contains(&opts.pos.len()) {
        let (least, most) = (spec.positionals.start(), spec.positionals.end());
        let want = if least == most { least.to_string() } else { format!("{least} to {most}") };
        return Err(CliError::Usage(format!(
            "{}: expected {want} argument(s), got {} (usage: {} {})",
            spec.name,
            opts.pos.len(),
            spec.name,
            spec.usage
        )));
    }
    Ok(opts)
}

fn is_help(a: &str) -> bool {
    a == "--help" || a == "-h"
}

/// What the line asks for: text to print, or a command to run.
enum Plan<'a> {
    Help(String),
    Run { dir: Option<&'a str>, spec: &'a Spec, opts: Opts },
}

fn plan_run<'a>(dir: Option<&'a str>, name: &str, rest: &[String]) -> Result<Plan<'a>, CliError> {
    let Some(spec) = SPECS.iter().find(|s| s.name == name) else {
        return Err(CliError::Usage(format!("unknown command {name:?} (try kerno --help)")));
    };
    if rest.iter().any(|a| is_help(a)) {
        return Ok(Plan::Help(command_help(spec)));
    }
    Ok(Plan::Run { dir, spec, opts: parse(rest, spec)? })
}

fn plan(args: &[String]) -> Result<Plan<'_>, CliError> {
    // `kerno import <file> <new-dir>` has no source directory. A file named
    // like a command is read as the directory of that command instead.
    if let [first, file, _] = args
        && first == "import"
        && !SPECS.iter().any(|s| s.name == file)
    {
        return plan_run(None, "import", &args[1..]);
    }
    match args {
        [] => Err(CliError::Usage("no database directory given (try kerno --help)".into())),
        [a, ..] if is_help(a) => Ok(Plan::Help(help())),
        [a, ..] if a.starts_with("--") => Err(CliError::Usage(format!("unknown option {a} (try kerno --help)"))),
        [_] => Err(CliError::Usage("no command given (try kerno --help)".into())),
        [_, name, ..] if is_help(name) => Ok(Plan::Help(help())),
        [dir, name, rest @ ..] => plan_run(Some(dir), name, rest),
    }
}

fn run(args: &[String]) -> Result<Output, CliError> {
    match plan(args)? {
        Plan::Help(text) => Ok(Output { text, json: Value::Null, failed: false }),
        Plan::Run { dir, spec, opts } => {
            let json = opts.has("--json");
            let dir = dir.map(Path::new);
            let mut out = match (spec.name, dir) {
                ("import", _) => cmd::import(&opts)?,
                ("verify", Some(d)) => cmd::verify(d, opts.has("--deep"))?,
                ("stats", Some(d)) => cmd::stats(d)?,
                ("get", Some(d)) => cmd::get(d, &opts)?,
                ("history", Some(d)) => cmd::history(d, &opts)?,
                ("relations", Some(d)) => cmd::relations(d, &opts)?,
                ("query", Some(d)) => cmd::query(d, &opts)?,
                ("export", Some(d)) => cmd::export(d, &opts)?,
                ("backup", Some(d)) => cmd::backup(d, &opts)?,
                ("ship", Some(d)) => cmd::ship(d, &opts)?,
                ("restore", Some(d)) => cmd::restore(d, &opts)?,
                ("backups", _) => cmd::backups(&opts)?,
                ("reclaim", Some(d)) => cmd::reclaim(d)?,
                (other, _) => return Err(CliError::Usage(format!("{other} needs a database directory first"))),
            };
            if json {
                out.text = serde_json::to_string_pretty(&out.json).unwrap_or_default();
            }
            Ok(out)
        }
    }
}

fn main() -> ExitCode {
    // `std::env::args()` panics on an argument that is not Unicode: say so in one line instead.
    let args: Result<Vec<String>, _> = std::env::args_os().skip(1).map(|a| a.into_string()).collect();
    let Ok(args) = args else {
        eprintln!("kerno: an argument is not valid Unicode");
        return ExitCode::from(2);
    };
    match run(&args) {
        Ok(out) => {
            let mut stdout = std::io::stdout().lock();
            // A closed pipe (`kerno ... | head`) is not an error.
            match writeln!(stdout, "{}", out.text).and_then(|()| stdout.flush()) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => {}
                Err(e) => {
                    eprintln!("kerno: cannot write the output: {e}");
                    return ExitCode::from(1);
                }
            }
            if out.failed { ExitCode::from(1) } else { ExitCode::SUCCESS }
        }
        Err(CliError::Usage(m)) => {
            eprintln!("kerno: {m}");
            ExitCode::from(2)
        }
        Err(CliError::Failure(m)) => {
            eprintln!("kerno: {m}");
            ExitCode::from(1)
        }
    }
}
