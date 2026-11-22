//! The command line of `kerno-server`.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

pub const USAGE: &str = "usage: kerno-server --db <dir> [options]\n\
    \x20      kerno-server --data-dir <dir> --tokens <file> [options]\n\
    \x20      kerno-server hash-token < token\n\
    --db <dir>            one database, no authentication (or set KERNO_SERVER_DB); created if missing\n\
    --data-dir <dir>      one database per tenant, in <dir>/<tenant>; needs --tokens\n\
    --tokens <file>       the API tokens: one `<sha256 hex> <tenant>` per line\n\
    --listen <addr>       the HTTP address; default 127.0.0.1:8080, and port 0 picks a free port\n\
    --grpc-listen <addr>  also serve gRPC on this address\n\
    --tls-cert <pem>      serve both transports over TLS with this certificate chain\n\
    --tls-key <pem>       and this private key (PKCS#8, PKCS#1 or SEC1)\n\
    --backup <url>        keep a base backup and the shipped write-ahead log of every database in <url>\n\
    \x20                     (file:///dir or s3://bucket/prefix; a tenant's backups go under its name)\n\
    --backup-interval <s> seconds between shipments; default 5\n\
    --fulltext <types>    unit types to keyword-index, comma-separated (off by default)\n\
    hash-token            print the SHA-256 of the token read on stdin, for the tokens file\n\
    Off Linux, durability is not verified: it starts anyway and logs a warning; set\n\
    KERNO_REQUIRE_VERIFIED_DURABILITY=1 to refuse to start there.";

/// Where the HTTP API listens unless `--listen` says otherwise: the loopback interface only, because
/// `--db` serves without authentication.
pub const DEFAULT_LISTEN: &str = "127.0.0.1:8080";

/// What the server serves, and who may call it.
#[derive(Debug, PartialEq, Eq)]
pub enum Store {
    /// One database, open to anyone who reaches the port.
    Db(PathBuf),
    /// One database per tenant under `root`; a request needs a token of `tokens`, which picks the
    /// tenant.
    Tenants { root: PathBuf, tokens: PathBuf },
}

#[derive(Debug, PartialEq, Eq)]
pub struct Tls {
    pub cert: PathBuf,
    pub key: PathBuf,
}

/// Where the backups go and how often the write-ahead log is shipped there.
#[derive(Debug, PartialEq, Eq)]
pub struct Backup {
    pub url: String,
    pub interval: Duration,
}

/// Seconds between shipments unless `--backup-interval` says otherwise.
pub const DEFAULT_BACKUP_INTERVAL: Duration = Duration::from_secs(5);

#[derive(Debug, PartialEq, Eq)]
pub struct Args {
    pub store: Store,
    pub listen: SocketAddr,
    /// gRPC is served only when an address is given.
    pub grpc_listen: Option<SocketAddr>,
    pub tls: Option<Tls>,
    pub backup: Option<Backup>,
    /// Unit types whose keyword index is turned on in every database served (ADR-0023:
    /// the index is off by default).
    pub fulltext: Vec<String>,
}

fn address(flag: &str, text: &str) -> Result<SocketAddr, String> {
    text.parse().map_err(|e| format!("{flag} {text:?}: {e}"))
}

/// The arguments after the program name, and `KERNO_SERVER_DB` as the fallback for `--db`.
/// `Ok(None)` means help was asked for.
pub fn parse(args: impl Iterator<Item = String>, env_db: Option<String>) -> Result<Option<Args>, String> {
    let (mut db, mut listen, mut grpc, mut data_dir, mut tokens, mut cert, mut key) =
        (None, None, None, None, None, None, None);
    let (mut backup, mut every): (Option<String>, Option<String>) = (None, None);
    let mut fulltext: Option<String> = None;
    let mut args = args;
    while let Some(arg) = args.next() {
        let (flag, inline) = match arg.split_once('=') {
            Some((flag, value)) if flag.starts_with("--") => (flag.to_owned(), Some(value.to_owned())),
            _ => (arg, None),
        };
        let slot = match flag.as_str() {
            "-h" | "--help" => return Ok(None),
            "--db" => &mut db,
            "--listen" => &mut listen,
            "--grpc-listen" => &mut grpc,
            "--data-dir" => &mut data_dir,
            "--tokens" => &mut tokens,
            "--tls-cert" => &mut cert,
            "--tls-key" => &mut key,
            "--backup" => &mut backup,
            "--backup-interval" => &mut every,
            "--fulltext" => &mut fulltext,
            other => return Err(format!("unknown argument {other:?}")),
        };
        *slot = Some(match inline {
            Some(value) => value,
            None => args.next().ok_or_else(|| format!("{flag} needs a value"))?,
        });
    }
    let store = match (db, data_dir, tokens) {
        (Some(_), Some(_), _) => return Err("--db and --data-dir cannot be used together".into()),
        (Some(_), None, Some(_)) => {
            return Err("--tokens needs --data-dir: --db serves one database without authentication".into());
        }
        (None, Some(root), Some(tokens)) => Store::Tenants { root: root.into(), tokens: tokens.into() },
        (None, Some(_), None) => return Err("--data-dir needs --tokens <file>".into()),
        (None, None, Some(_)) => return Err("--tokens needs --data-dir <dir>".into()),
        (db, None, None) => Store::Db(
            db.or(env_db)
                .filter(|d| !d.is_empty())
                .ok_or("no database: pass --db <dir>, --data-dir <dir> with --tokens <file>, or set KERNO_SERVER_DB")?
                .into(),
        ),
    };
    let tls = match (cert, key) {
        (Some(cert), Some(key)) => Some(Tls { cert: cert.into(), key: key.into() }),
        (None, None) => None,
        _ => return Err("--tls-cert and --tls-key go together".into()),
    };
    let backup = match (backup, every) {
        (None, None) => None,
        (None, Some(_)) => return Err("--backup-interval needs --backup <url>".into()),
        (Some(url), every) => {
            let interval = match every {
                None => DEFAULT_BACKUP_INTERVAL,
                Some(text) => match text.parse::<u64>() {
                    Ok(secs) if secs >= 1 => Duration::from_secs(secs),
                    _ => return Err(format!("--backup-interval {text:?} is not a whole number of seconds, 1 or more")),
                },
            };
            Some(Backup { url, interval })
        }
    };
    Ok(Some(Args {
        store,
        listen: address("--listen", listen.as_deref().unwrap_or(DEFAULT_LISTEN))?,
        grpc_listen: grpc.as_deref().map(|a| address("--grpc-listen", a)).transpose()?,
        tls,
        backup,
        fulltext: fulltext
            .map(|t| t.split(',').map(str::trim).filter(|t| !t.is_empty()).map(str::to_owned).collect())
            .unwrap_or_default(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_of(args: &[&str], env: Option<&str>) -> Result<Option<Args>, String> {
        parse(args.iter().map(|a| (*a).to_owned()), env.map(str::to_owned))
    }

    #[test]
    fn the_defaults_are_loopback_http_only() {
        let args = parse_of(&["--db", "/data"], None).unwrap().unwrap();
        assert_eq!(args.store, Store::Db(PathBuf::from("/data")));
        assert_eq!(args.listen, "127.0.0.1:8080".parse().unwrap());
        assert!(args.listen.ip().is_loopback());
        assert_eq!(args.grpc_listen, None);
        assert_eq!(args.tls, None);
    }

    #[test]
    fn a_flag_takes_its_value_after_a_space_or_an_equals_sign() {
        let args = parse_of(&["--db=/d", "--listen=0.0.0.0:9", "--grpc-listen", "127.0.0.1:0"], None).unwrap().unwrap();
        assert_eq!(args.store, Store::Db(PathBuf::from("/d")));
        assert_eq!(args.listen, "0.0.0.0:9".parse().unwrap());
        assert_eq!(args.grpc_listen, Some("127.0.0.1:0".parse().unwrap()));
    }

    #[test]
    fn the_environment_names_the_database_and_the_flag_wins() {
        let store = |args: &[&str], env| parse_of(args, env).unwrap().unwrap().store;
        assert_eq!(store(&[], Some("/env")), Store::Db(PathBuf::from("/env")));
        assert_eq!(store(&["--db", "/flag"], Some("/env")), Store::Db(PathBuf::from("/flag")));
        // The environment never turns tenant mode into one open database.
        assert_eq!(
            store(&["--data-dir", "/t", "--tokens", "/k"], Some("/env")),
            Store::Tenants { root: PathBuf::from("/t"), tokens: PathBuf::from("/k") }
        );
        assert!(parse_of(&[], Some("")).unwrap_err().contains("no database"));
        assert!(parse_of(&[], None).unwrap_err().contains("no database"));
    }

    #[test]
    fn bad_arguments_are_named() {
        assert!(parse_of(&["--db"], None).unwrap_err().contains("--db needs a value"));
        assert!(parse_of(&["--db", "/d", "--nope"], None).unwrap_err().contains("--nope"));
        assert!(parse_of(&["--db", "/d", "--listen", "localhost"], None).unwrap_err().contains("--listen"));
        assert!(parse_of(&["--db", "/d", "--grpc-listen=x"], None).unwrap_err().contains("--grpc-listen"));
    }

    #[test]
    fn tenant_mode_needs_both_a_data_directory_and_tokens() {
        let err = |args: &[&str]| parse_of(args, None).unwrap_err();
        assert_eq!(err(&["--data-dir", "/t"]), "--data-dir needs --tokens <file>");
        assert_eq!(err(&["--tokens", "/k"]), "--tokens needs --data-dir <dir>");
        assert_eq!(
            err(&["--db", "/d", "--data-dir", "/t", "--tokens", "/k"]),
            "--db and --data-dir cannot be used together"
        );
        assert_eq!(err(&["--db", "/d", "--data-dir", "/t"]), "--db and --data-dir cannot be used together");
        assert!(err(&["--db", "/d", "--tokens", "/k"]).starts_with("--tokens needs --data-dir: --db serves"));
    }

    #[test]
    fn tls_needs_both_a_certificate_and_a_key() {
        let args = parse_of(&["--db", "/d", "--tls-cert", "/c.pem", "--tls-key=/k.pem"], None).unwrap().unwrap();
        assert_eq!(args.tls, Some(Tls { cert: PathBuf::from("/c.pem"), key: PathBuf::from("/k.pem") }));
        for half in [["--tls-cert", "/c.pem"], ["--tls-key", "/k.pem"]] {
            let args = ["--db", "/d", half[0], half[1]];
            assert_eq!(parse_of(&args, None).unwrap_err(), "--tls-cert and --tls-key go together");
        }
    }

    #[test]
    fn backups_are_off_unless_asked_for_and_ship_every_five_seconds_by_default() {
        assert_eq!(parse_of(&["--db", "/d"], None).unwrap().unwrap().backup, None);
        let backup = |args: &[&str]| parse_of(args, None).unwrap().unwrap().backup.unwrap();
        let on = backup(&["--db", "/d", "--backup", "file:///b"]);
        assert_eq!((on.url.as_str(), on.interval), ("file:///b", Duration::from_secs(5)));
        let on = backup(&["--db", "/d", "--backup=s3://bucket/p", "--backup-interval", "30"]);
        assert_eq!((on.url.as_str(), on.interval), ("s3://bucket/p", Duration::from_secs(30)));
    }

    #[test]
    fn the_backup_flags_go_together_and_the_interval_is_a_whole_number_of_seconds() {
        let err = |args: &[&str]| parse_of(args, None).unwrap_err();
        assert_eq!(err(&["--db", "/d", "--backup-interval", "3"]), "--backup-interval needs --backup <url>");
        for bad in ["0", "-1", "1.5", "soon", ""] {
            let message = err(&["--db", "/d", "--backup", "file:///b", "--backup-interval", bad]);
            assert_eq!(message, format!("--backup-interval {bad:?} is not a whole number of seconds, 1 or more"));
        }
        assert!(err(&["--db", "/d", "--backup"]).contains("--backup needs a value"));
    }

    #[test]
    fn help_is_not_an_error() {
        assert_eq!(parse_of(&["--help"], None), Ok(None));
        assert_eq!(parse_of(&["--db", "/d", "-h"], None), Ok(None));
    }
}
