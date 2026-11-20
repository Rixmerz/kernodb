//! The HTTP JSON transport: a router over `hyper`, one [`Service`] call per route.
//!
//! Every answer but `GET /metrics` (Prometheus text) is JSON. A failure is
//! `{"error": {"code", "message"}}` with the status the [`Kind`] fixes, including the ones hyper
//! would answer without a body (unknown route, wrong method, body too large). A body that does not
//! parse is a 400 that names the problem.
//!
//! `/livez`, `/readyz` and `/metrics` need no token; every `/v1` route does when the server has
//! tokens, checked before the route or the body is looked at.

use std::convert::Infallible;
use std::future::Future;
use std::sync::Arc;
use std::time::{Duration, Instant};

use bytes::Bytes;
use http_body_util::{BodyExt, Full, LengthLimitError, Limited};
use hyper::body::Incoming;
use hyper::header::{ALLOW, AUTHORIZATION, CONTENT_TYPE, HeaderValue, WWW_AUTHENTICATE};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::{TokioIo, TokioTimer};
use hyper_util::server::graceful::{GracefulShutdown, Watcher};
use percent_encoding::percent_decode_str;
use serde::Serialize;
use serde::de::DeserializeOwned;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;

use crate::MAX_BODY;
use crate::api::{
    CreateRelation, CreateUnit, Expect, ListRelations, ListUnits, QueryRequest, SearchRequest, UpdateUnit,
};
use crate::app::App;
use crate::error::{ApiError, Kind};
use crate::service::{Res, Service};

/// How long a client may take to send the request line and headers, and to finish a TLS handshake.
const HEADER_TIMEOUT: Duration = Duration::from_secs(10);
/// How long shutdown waits for requests in flight.
const GRACE: Duration = Duration::from_secs(10);

type Reply = Response<Full<Bytes>>;

/// Serves `listener`, over TLS when `tls` is given, until `stop` completes, then lets the requests
/// in flight finish.
///
/// ponytail: no timeout on a request body once the headers are in; a reverse proxy in front adds
/// one if slow clients become a problem.
pub async fn serve(listener: TcpListener, app: Arc<App>, tls: Option<TlsAcceptor>, stop: impl Future<Output = ()>) {
    let graceful = GracefulShutdown::new();
    let mut stop = std::pin::pin!(stop);
    loop {
        tokio::select! {
            accepted = listener.accept() => {
                let Ok((stream, _)) = accepted else {
                    // Out of file descriptors, most likely: do not spin while it lasts.
                    tokio::time::sleep(Duration::from_millis(10)).await;
                    continue;
                };
                let (app, tls, watcher) = (Arc::clone(&app), tls.clone(), graceful.watcher());
                // The handshake runs in the connection's own task: a slow client holds up no one.
                tokio::spawn(async move {
                    let Some(acceptor) = tls else { return connection(watcher, stream, app).await };
                    match tokio::time::timeout(HEADER_TIMEOUT, acceptor.accept(stream)).await {
                        Ok(Ok(stream)) => connection(watcher, stream, app).await,
                        Ok(Err(e)) => tracing::debug!(error = %e, "TLS handshake failed"),
                        Err(_) => tracing::debug!("TLS handshake timed out"),
                    }
                });
            }
            () = &mut stop => break,
        }
    }
    // A client that never finishes must not keep the server from stopping.
    let _ = tokio::time::timeout(GRACE, graceful.shutdown()).await;
}

async fn connection(watcher: Watcher, io: impl AsyncRead + AsyncWrite + Unpin + Send + 'static, app: Arc<App>) {
    let conn = http1::Builder::new()
        .timer(TokioTimer::new())
        .header_read_timeout(HEADER_TIMEOUT)
        .serve_connection(TokioIo::new(io), service_fn(move |req| handle(Arc::clone(&app), req)));
    let _ = watcher.watch(conn).await;
}

/// Answers one request, counts it and logs it.
async fn handle(app: Arc<App>, req: Request<Incoming>) -> Result<Reply, Infallible> {
    let start = Instant::now();
    let (method, path) = (req.method().clone(), req.uri().path().to_owned());
    let mut tenant = None;
    let reply = route(&app, req, &mut tenant).await.unwrap_or_else(|e| error_reply(&e));
    let took = start.elapsed();
    let status = reply.status().as_u16();
    app.metrics.http(status, took);
    tracing::info!(
        transport = "http",
        %method,
        path,
        status,
        tenant,
        duration_us = u64::try_from(took.as_micros()).unwrap_or(u64::MAX),
        "request"
    );
    Ok(reply)
}

fn reply(status: StatusCode, body: Vec<u8>) -> Reply {
    let mut r = Response::new(Full::new(Bytes::from(body)));
    *r.status_mut() = status;
    r.headers_mut().insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    r
}

fn error_reply(e: &ApiError) -> Reply {
    let status = StatusCode::from_u16(e.kind.http_status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let mut r = reply(status, e.body());
    if let Some(allow) = e.allow {
        r.headers_mut().insert(ALLOW, HeaderValue::from_static(allow));
    }
    if e.kind == Kind::Unauthenticated {
        r.headers_mut().insert(WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"));
    }
    r
}

fn json<T: Serialize>(status: StatusCode, value: &T) -> Res<Reply> {
    let body = serde_json::to_vec(value).map_err(|e| ApiError::internal(format!("cannot encode the answer: {e}")))?;
    Ok(reply(status, body))
}

fn ok<T: Serialize>(value: &T) -> Res<Reply> {
    json(StatusCode::OK, value)
}

fn created<T: Serialize>(value: &T) -> Res<Reply> {
    json(StatusCode::CREATED, value)
}

fn no_content() -> Res<Reply> {
    let mut r = Response::new(Full::new(Bytes::new()));
    *r.status_mut() = StatusCode::NO_CONTENT;
    Ok(r)
}

async fn read_body(body: Incoming) -> Res<Bytes> {
    match Limited::new(body, MAX_BODY).collect().await {
        Ok(all) => Ok(all.to_bytes()),
        Err(e) if e.is::<LengthLimitError>() => {
            Err(ApiError::new(Kind::PayloadTooLarge, format!("the request body is over {MAX_BODY} bytes")))
        }
        Err(e) => Err(ApiError::invalid(format!("cannot read the request body: {e}"))),
    }
}

/// The JSON body of a request, as a `T`.
async fn body_of<T: DeserializeOwned>(body: Incoming) -> Res<T> {
    let bytes = read_body(body).await?;
    serde_json::from_slice(&bytes).map_err(|e| ApiError::invalid(format!("invalid request body: {e}")))
}

/// The query string of a request, as a `T` whose members are all optional.
fn params<T: DeserializeOwned>(query: Option<&str>) -> Res<T> {
    serde_urlencoded::from_str(query.unwrap_or("")).map_err(|e| ApiError::invalid(format!("invalid query string: {e}")))
}

/// The path split at `/`, each segment percent-decoded (a unit type may hold any UTF-8).
fn segments(path: &str) -> Res<Vec<String>> {
    path.trim_matches('/')
        .split('/')
        .map(|s| {
            percent_decode_str(s)
                .decode_utf8()
                .map(|decoded| decoded.into_owned())
                .map_err(|_| ApiError::invalid("the path is not valid UTF-8 once percent-decoded"))
        })
        .collect()
}

/// The probes and the metrics, open to anyone; then the API, behind the token when there are
/// tokens. `tenant` is set once a token is accepted, for the log.
async fn route(app: &App, req: Request<Incoming>, tenant: &mut Option<String>) -> Res<Reply> {
    let (parts, body) = req.into_parts();
    let decoded = segments(parts.uri.path())?;
    let segs: Vec<&str> = decoded.iter().map(String::as_str).collect();
    let get = parts.method == Method::GET;
    match segs.as_slice() {
        ["livez"] if get => ok(&serde_json::json!({ "status": "alive" })),
        ["readyz"] if get => {
            app.ready().await?;
            ok(&serde_json::json!({ "status": "ready" }))
        }
        ["metrics"] if get => {
            let mut r = Response::new(Full::new(Bytes::from(app.metrics.render(app.databases(), app.backup_totals()))));
            r.headers_mut().insert(CONTENT_TYPE, HeaderValue::from_static("text/plain; version=0.0.4"));
            Ok(r)
        }
        ["livez" | "readyz" | "metrics"] => Err(ApiError::method_not_allowed("GET")),
        ["v1", ..] => {
            let (name, svc) = app.service(parts.headers.get(AUTHORIZATION).map(HeaderValue::as_bytes))?;
            if app.authenticates() {
                *tenant = Some(name.to_owned());
            }
            api(&svc, &parts, body, &segs).await
        }
        _ => Err(ApiError::not_found(format!("no route {} {}", parts.method, parts.uri.path()))),
    }
}

async fn api(svc: &Arc<Service>, parts: &hyper::http::request::Parts, body: Incoming, segs: &[&str]) -> Res<Reply> {
    let query = parts.uri.query();
    let not_allowed = ApiError::method_not_allowed;

    match (&parts.method, segs) {
        (&Method::GET, ["v1", "health"]) => ok(&svc.run(|s| s.health()).await?),
        (_, ["v1", "health"]) => Err(not_allowed("GET")),

        (&Method::POST, ["v1", "units"]) => {
            let req: CreateUnit = body_of(body).await?;
            created(&svc.run(move |s| s.create_unit(req)).await?)
        }
        (&Method::GET, ["v1", "units"]) => {
            let req: ListUnits = params(query)?;
            ok(&svc.run(move |s| s.list_units(req)).await?)
        }
        (_, ["v1", "units"]) => Err(not_allowed("GET, POST")),

        (&Method::GET, ["v1", "units", id]) => {
            let id = (*id).to_owned();
            ok(&svc.run(move |s| s.get_unit(&id)).await?)
        }
        (&Method::PUT, ["v1", "units", id]) => {
            let (id, expect) = ((*id).to_owned(), params::<Expect>(query)?);
            let req: UpdateUnit = body_of(body).await?;
            ok(&svc.run(move |s| s.update_unit(&id, req, expect.expected_revision)).await?)
        }
        (&Method::DELETE, ["v1", "units", id]) => {
            let (id, expect) = ((*id).to_owned(), params::<Expect>(query)?);
            svc.run(move |s| s.delete_unit(&id, expect.expected_revision)).await?;
            no_content()
        }
        (_, ["v1", "units", _]) => Err(not_allowed("DELETE, GET, PUT")),

        (&Method::POST, ["v1", "units", id, "archive"]) => {
            let (id, expect) = ((*id).to_owned(), params::<Expect>(query)?);
            ok(&svc.run(move |s| s.archive_unit(&id, expect.expected_revision)).await?)
        }
        (_, ["v1", "units", _, "archive"]) => Err(not_allowed("POST")),

        (&Method::POST, ["v1", "units", id, "unarchive"]) => {
            let (id, expect) = ((*id).to_owned(), params::<Expect>(query)?);
            ok(&svc.run(move |s| s.unarchive_unit(&id, expect.expected_revision)).await?)
        }
        (_, ["v1", "units", _, "unarchive"]) => Err(not_allowed("POST")),

        (&Method::GET, ["v1", "units", id, "relations"]) => {
            let (id, req) = ((*id).to_owned(), params::<ListRelations>(query)?);
            ok(&svc.run(move |s| s.list_relations(&id, req)).await?)
        }
        (_, ["v1", "units", _, "relations"]) => Err(not_allowed("GET")),

        (&Method::POST, ["v1", "backup"]) => created(&svc.run(|s| s.backup_now()).await?),
        (_, ["v1", "backup"]) => Err(not_allowed("POST")),

        (&Method::POST, ["v1", "relations"]) => {
            let req: CreateRelation = body_of(body).await?;
            created(&svc.run(move |s| s.create_relation(req)).await?)
        }
        (_, ["v1", "relations"]) => Err(not_allowed("POST")),

        (&Method::DELETE, ["v1", "relations", id]) => {
            let id = (*id).to_owned();
            svc.run(move |s| s.delete_relation(&id)).await?;
            no_content()
        }
        (_, ["v1", "relations", _]) => Err(not_allowed("DELETE")),

        (&Method::POST, ["v1", "schemas"]) => {
            let req = body_of(body).await?;
            created(&svc.run(move |s| s.define_schema(req)).await?)
        }
        (_, ["v1", "schemas"]) => Err(not_allowed("POST")),

        (&Method::GET, ["v1", "schemas", unit_type]) => {
            let unit_type = (*unit_type).to_owned();
            ok(&svc.run(move |s| s.get_schema(&unit_type)).await?)
        }
        (_, ["v1", "schemas", _]) => Err(not_allowed("GET")),

        (&Method::POST, ["v1", "query"]) => {
            let req: QueryRequest = body_of(body).await?;
            ok(&svc.run(move |s| s.query(req)).await?)
        }
        (_, ["v1", "query"]) => Err(not_allowed("POST")),

        (&Method::POST, ["v1", "query", "explain"]) => {
            let req: QueryRequest = body_of(body).await?;
            ok(&svc.run(move |s| s.explain(req)).await?)
        }
        (_, ["v1", "query", "explain"]) => Err(not_allowed("POST")),

        (&Method::POST, ["v1", "search"]) => {
            let req: SearchRequest = body_of(body).await?;
            ok(&svc.run(move |s| s.search(req)).await?)
        }
        (_, ["v1", "search"]) => Err(not_allowed("POST")),

        _ => Err(ApiError::not_found(format!("no route {} {}", parts.method, parts.uri.path()))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_path_splits_into_decoded_segments() {
        assert_eq!(segments("/v1/schemas/my%20type/").unwrap(), ["v1", "schemas", "my type"]);
        assert_eq!(segments("/v1/schemas/caf%C3%A9").unwrap(), ["v1", "schemas", "café"]);
        assert_eq!(segments("/").unwrap(), [""]);
        // Bytes that are not UTF-8 once decoded.
        assert!(segments("/v1/%ff").is_err());
    }
}
