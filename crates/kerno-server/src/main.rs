//! `kerno-server --db <dir>`: see the crate README and `docs/server.md`.

use std::future::Future;
use std::io::Read;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use kerno_server::app::App;
use kerno_server::args::{self, Args, USAGE};
use kerno_server::auth::{MIN_TOKEN_LEN, hash_hex};
use kerno_server::{grpc, http, tls};
use tokio::net::TcpListener;
use tokio::sync::watch;

/// Completes when the process is asked to stop: Ctrl-C, or SIGTERM where there is one.
///
/// The handlers are installed when this is called, not when it is first polled: called before the
/// ports are printed, a SIGTERM sent the moment they are read stops the server gracefully instead
/// of killing it.
fn stop_signal() -> impl Future<Output = ()> {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        let handlers = signal(SignalKind::interrupt()).and_then(|i| Ok((i, signal(SignalKind::terminate())?)));
        async move {
            match handlers {
                Ok((mut interrupt, mut term)) => {
                    tokio::select! {
                        _ = interrupt.recv() => {}
                        _ = term.recv() => {}
                    }
                }
                Err(_) => {
                    let _ = tokio::signal::ctrl_c().await;
                }
            }
        }
    }
    #[cfg(not(unix))]
    async {
        let _ = tokio::signal::ctrl_c().await;
    }
}

/// Ships the backups every `every`, the first time at once, until the server is told to stop.
async fn backup_loop(app: Arc<App>, every: Duration, mut stop: watch::Receiver<()>) {
    let mut timer = tokio::time::interval(every);
    timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            _ = timer.tick() => app.backup_tick().await,
            _ = stop.changed() => break,
        }
    }
}

async fn bind(addr: std::net::SocketAddr) -> Result<TcpListener, String> {
    TcpListener::bind(addr).await.map_err(|e| format!("cannot listen on {addr}: {e}"))
}

/// Serves until a stop signal, then returns once the requests in flight are done.
async fn serve(app: &Arc<App>, args: &Args) -> Result<(), String> {
    let tls = args.tls.as_ref().map(tls::load).transpose()?;
    let stop = stop_signal();
    let http = bind(args.listen).await?;
    let grpc = match args.grpc_listen {
        Some(addr) => Some(bind(addr).await?),
        None => None,
    };
    // Tests and scripts read these lines on stdout to find the ports; port 0 means "pick one".
    // The logs go to stderr.
    let local = |l: &TcpListener| l.local_addr().map_err(|e| e.to_string());
    let scheme = if tls.is_some() { "https" } else { "http" };
    println!("listening on {scheme}://{}", local(&http)?);
    if let Some(l) = &grpc {
        println!("grpc listening on {}", local(l)?);
    }
    let exposed = !args.listen.ip().is_loopback() || args.grpc_listen.is_some_and(|a| !a.ip().is_loopback());
    tracing::info!(
        http = %local(&http)?,
        grpc = grpc.as_ref().map(local).transpose()?.map(|a| a.to_string()),
        tls = tls.is_some(),
        authentication = app.authenticates(),
        databases = app.databases(),
        "listening"
    );
    if exposed && !app.authenticates() {
        tracing::warn!("serving beyond loopback without authentication: anyone who reaches the port has the database");
    } else if exposed && tls.is_none() {
        tracing::warn!("serving beyond loopback without TLS: tokens and data cross the network in clear text");
    }

    let (stop_tx, stop_rx) = watch::channel(());
    let stop_tx = Arc::new(stop_tx);
    let on_signal = Arc::clone(&stop_tx);
    tokio::spawn(async move {
        stop.await;
        tracing::info!("stopping");
        let _ = on_signal.send(());
    });
    let stopped = |mut rx: watch::Receiver<()>| async move {
        let _ = rx.changed().await;
    };
    let (http_tls, grpc_tls) = tls.map_or((None, None), |t| (Some(t.http), Some(t.grpc)));
    let http_task = http::serve(http, Arc::clone(app), http_tls, stopped(stop_rx.clone()));
    let grpc_task = async {
        let Some(listener) = grpc else { return Ok(()) };
        let served = grpc::serve(listener, Arc::clone(app), grpc_tls, stopped(stop_rx.clone())).await;
        // A gRPC server that ended, with an error or without one, takes the HTTP server down too.
        let _ = stop_tx.send(());
        served.map_err(|e| format!("the gRPC server failed: {e}"))
    };
    let backup_task = async {
        if let Some(every) = app.backup_interval() {
            backup_loop(Arc::clone(app), every, stop_rx.clone()).await;
        }
    };
    let ((), grpc_result, ()) = tokio::join!(http_task, grpc_task, backup_task);
    // Every request is answered, so this ships the last commits before the databases close.
    if app.backup_interval().is_some() {
        app.backup_tick().await;
    }
    grpc_result
}

/// `kerno-server hash-token`: the line of the tokens file for the token on stdin, less the tenant.
fn hash_token() -> Result<(), String> {
    let mut input = String::new();
    std::io::stdin().read_to_string(&mut input).map_err(|e| format!("cannot read the token on stdin: {e}"))?;
    let token = input.trim();
    if token.chars().count() < MIN_TOKEN_LEN || token.contains(char::is_whitespace) {
        return Err(format!(
            "a token is at least {MIN_TOKEN_LEN} characters with no white space; `openssl rand -hex 32` makes one"
        ));
    }
    println!("{}", hash_hex(token));
    Ok(())
}

async fn run(argv: Vec<String>) -> Result<(), String> {
    let parsed =
        args::parse(argv.into_iter(), std::env::var("KERNO_SERVER_DB").ok()).map_err(|e| format!("{e}\n{USAGE}"))?;
    let Some(args) = parsed else {
        println!("{USAGE}");
        return Ok(());
    };
    tracing_subscriber::fmt().json().with_writer(std::io::stderr).with_current_span(false).init();
    let app = Arc::new(App::open(&args.store, args.backup.as_ref(), &args.fulltext)?);
    let served = serve(&app, &args).await;
    // Every transport has stopped, so this is the last handle unless a handler thread is still
    // finishing; dropping it closes the databases too, only without a report.
    let closed = match Arc::try_unwrap(app) {
        Ok(app) => app.close(),
        Err(_) => Ok(()),
    };
    served?;
    closed
}

#[tokio::main]
async fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let result = if argv.first().map(String::as_str) == Some("hash-token") && argv.len() == 1 {
        hash_token()
    } else {
        run(argv).await
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            // Before the logger is up (a usage error) this is the only line; after, it is the
            // last one, and plain text so a person reading the terminal sees why it stopped.
            eprintln!("kerno-server: {e}");
            ExitCode::FAILURE
        }
    }
}
