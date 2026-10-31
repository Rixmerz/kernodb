//! Re-executes the running test binary as a child process, for tests that
//! need a second real process: the cross-process lock, or `kill -9` of a
//! writer.
//!
//! A test that wants a child calls [`spawn_child`] with its own path; the
//! child runs exactly that test again, and the test tells the two roles
//! apart with [`child_role`]:
//!
//! ```ignore
//! #[test]
//! fn two_processes() {
//!     if let Some(role) = kerno_testkit::process::child_role() {
//!         assert_eq!(role, "writer");
//!         let args = kerno_testkit::process::child_args();
//!         // ... child work; talk to the parent with `report` ...
//!         return;
//!     }
//!     let child = kerno_testkit::process::spawn_child("two_processes", "writer", &["/tmp/db"]);
//!     // ... parent work: read reports with `next_report`, kill or wait ...
//! }
//! ```
//!
//! The child inherits the environment, gets a piped stdin and stdout (its
//! stderr is the parent's), and runs with `--exact <test_path> --nocapture
//! --test-threads=1`. Its stdout also carries the test runner's own output,
//! so messages to the parent go through [`report`] and are read back with
//! [`next_report`].

use std::io::{BufRead, Write};
use std::process::{Child, Command, Stdio};

/// Environment variable holding the child's role.
pub const CHILD_ENV: &str = "KERNO_CHILD";
/// Environment variable holding the child's arguments (see [`child_args`]).
pub const CHILD_ARGS_ENV: &str = "KERNO_CHILD_ARGS";
/// Prefix of the lines [`report`] writes.
pub const REPORT_PREFIX: &str = "KERNO_CHILD> ";

/// Terminates each argument in [`CHILD_ARGS_ENV`] (ASCII unit separator).
const ARG_END: char = '\u{1f}';

/// Starts the running test binary again, running only the test at
/// `test_path` (its full path inside the binary, e.g. `process_lock` for a
/// top-level test of an integration-test file, or `module::tests::name`),
/// with [`child_role`] returning `role` and [`child_args`] returning `args`.
///
/// Panics if the binary cannot be started or an argument contains the
/// ASCII unit separator (`\x1f`).
pub fn spawn_child(test_path: &str, role: &str, args: &[&str]) -> Child {
    assert!(!args.iter().any(|a| a.contains(ARG_END)), "child arguments must not contain \\x1f");
    let encoded: String = args.iter().flat_map(|a| a.chars().chain([ARG_END])).collect();
    let exe = std::env::current_exe().expect("path of the running test binary");
    Command::new(&exe)
        .args(["--exact", test_path, "--nocapture", "--test-threads=1"])
        .env(CHILD_ENV, role)
        .env(CHILD_ARGS_ENV, encoded)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap_or_else(|e| panic!("cannot re-execute {} as a child: {e}", exe.display()))
}

/// The role this process was started with by [`spawn_child`], or `None` in
/// the parent (a normal test run).
pub fn child_role() -> Option<String> {
    std::env::var(CHILD_ENV).ok()
}

/// The arguments this process was started with by [`spawn_child`] (empty in
/// the parent).
pub fn child_args() -> Vec<String> {
    let encoded = std::env::var(CHILD_ARGS_ENV).unwrap_or_default();
    let mut args: Vec<String> = encoded.split(ARG_END).map(str::to_owned).collect();
    args.pop(); // after the last terminator
    args
}

/// In a child: sends one line to the parent (see [`next_report`]).
pub fn report(msg: &str) {
    assert!(!msg.contains('\n'), "a report is a single line");
    let mut out = std::io::stdout().lock();
    writeln!(out, "{REPORT_PREFIX}{msg}").and_then(|()| out.flush()).expect("write to the parent");
}

/// In the parent: the next line the child sent with [`report`], skipping the
/// test runner's own output; `None` once the child's stdout is closed.
///
/// The report may follow other text on the same line: with one test thread
/// the runner prints `test <name> ... ` before running the test and ends
/// that line only when the test finishes.
pub fn next_report(child_stdout: &mut impl BufRead) -> Option<String> {
    let mut line = String::new();
    loop {
        line.clear();
        if child_stdout.read_line(&mut line).expect("read the child's stdout") == 0 {
            return None;
        }
        if let Some(at) = line.find(REPORT_PREFIX) {
            return Some(line[at + REPORT_PREFIX.len()..].trim_end_matches(['\r', '\n']).to_owned());
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::{BufReader, Read};

    use super::*;

    #[test]
    fn child_process_round_trip() {
        if let Some(role) = child_role() {
            assert_eq!(role, "echo");
            report(&format!("args={:?}", child_args()));
            let mut line = String::new();
            std::io::stdin().read_line(&mut line).unwrap();
            report(&format!("got={}", line.trim_end()));
            return;
        }
        assert!(child_args().is_empty());
        let mut child = spawn_child("process::tests::child_process_round_trip", "echo", &["a b", "", "c"]);
        let mut out = BufReader::new(child.stdout.take().unwrap());
        assert_eq!(next_report(&mut out).as_deref(), Some(r#"args=["a b", "", "c"]"#));
        writeln!(child.stdin.take().unwrap(), "ping").unwrap();
        assert_eq!(next_report(&mut out).as_deref(), Some("got=ping"));
        let mut rest = String::new();
        out.read_to_string(&mut rest).unwrap();
        assert!(child.wait().unwrap().success());
        assert!(rest.contains("1 passed"), "the child ran exactly this test: {rest}");
        // A child of a test that does not exist runs nothing.
        let mut none = spawn_child("process::tests::no_such_test", "echo", &[]);
        assert_eq!(next_report(&mut BufReader::new(none.stdout.take().unwrap())), None);
        assert!(none.wait().unwrap().success());
    }
}
