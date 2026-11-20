//! The gRPC transport: `proto/kerno/v1/kerno.proto`, every RPC a call of the same [`Service`]
//! the HTTP routes call. A message converts to the request type of [`crate::api`] and the answer
//! converts back, so the two transports cannot disagree on what an operation does.

use std::collections::{BTreeMap, HashMap};
use std::future::Future;
use std::sync::Arc;
use std::time::Instant;

use kerno::FieldValue;
use tokio::net::TcpListener;
use tokio_stream::wrappers::TcpListenerStream;
use tonic::metadata::MetadataValue;
use tonic::transport::ServerTlsConfig;
use tonic::{Code, Request, Response, Status};

use crate::api::{self, Field, Hex};
use crate::app::App;
use crate::error::{ApiError, Kind};
use crate::service::{Res, Service};

/// The generated messages, server and client.
#[allow(clippy::all, clippy::pedantic, clippy::unwrap_used, clippy::expect_used, clippy::panic)]
pub mod pb {
    tonic::include_proto!("kerno.v1");
}

use pb::kerno_server::{Kerno, KernoServer};

/// The metadata key that carries the machine-readable `code` of an error, the same one the HTTP
/// body carries.
pub const ERROR_CODE_KEY: &str = "kerno-error-code";

impl From<ApiError> for Status {
    fn from(e: ApiError) -> Status {
        let code = match e.kind {
            Kind::InvalidRequest | Kind::SchemaViolation => Code::InvalidArgument,
            Kind::Unauthenticated => Code::Unauthenticated,
            Kind::NotFound => Code::NotFound,
            Kind::MethodNotAllowed => Code::Unimplemented,
            Kind::RevisionConflict => Code::Aborted,
            Kind::Conflict => Code::FailedPrecondition,
            Kind::PayloadTooLarge => Code::ResourceExhausted,
            Kind::Unavailable => Code::Unavailable,
            Kind::Internal => Code::Internal,
        };
        let mut status = Status::new(code, e.message);
        if let Ok(value) = MetadataValue::try_from(e.kind.code()) {
            status.metadata_mut().insert(ERROR_CODE_KEY, value);
        }
        status
    }
}

/// The metadata key of the token, as on HTTP: `authorization: Bearer <token>`.
pub const AUTHORIZATION_KEY: &str = "authorization";

/// Serves `listener`, over TLS when `tls` is given, until `stop` completes, then lets the calls in
/// flight finish.
pub async fn serve(
    listener: TcpListener,
    app: Arc<App>,
    tls: Option<ServerTlsConfig>,
    stop: impl Future<Output = ()>,
) -> Result<(), tonic::transport::Error> {
    let service = KernoServer::new(Grpc(app)).max_decoding_message_size(crate::MAX_BODY);
    let mut server = tonic::transport::Server::builder();
    if let Some(tls) = tls {
        server = server.tls_config(tls)?;
    }
    server.serve_with_incoming_shutdown(service, TcpListenerStream::new(listener), stop).await
}

struct Grpc(Arc<App>);

impl Grpc {
    /// The database of the call's token. Every RPC asks first, before it reads its message.
    fn svc<T>(&self, request: &Request<T>) -> Res<Arc<Service>> {
        let authorization = request.metadata().get(AUTHORIZATION_KEY).map(|v| v.as_bytes());
        Ok(self.0.service(authorization)?.1)
    }

    /// Runs one RPC, then counts and logs it.
    async fn observed<T>(&self, method: &'static str, call: impl Future<Output = Res<T>>) -> Result<T, Status> {
        let start = Instant::now();
        let out = call.await;
        let took = start.elapsed();
        let code = out.as_ref().map_or_else(|e| e.kind.code(), |_| "ok");
        self.0.metrics.grpc(code, took);
        let duration_us = u64::try_from(took.as_micros()).unwrap_or(u64::MAX);
        tracing::info!(transport = "grpc", method, code, duration_us, "request");
        out.map_err(Status::from)
    }
}

async fn call<T: Send + 'static>(svc: &Arc<Service>, work: impl FnOnce(&Service) -> Res<T> + Send + 'static) -> Res<T> {
    svc.run(work).await
}

#[tonic::async_trait]
impl Kerno for Grpc {
    async fn health(&self, request: Request<pb::Empty>) -> Result<Response<pb::HealthReply>, Status> {
        self.observed("health", async move {
            let svc = self.svc(&request)?;
            let health = call(&svc, |s| s.health()).await?;
            Ok(Response::new(pb::HealthReply { status: health.status.to_owned(), version: health.version.to_owned() }))
        })
        .await
    }

    async fn create_unit(&self, request: Request<pb::CreateUnitRequest>) -> Result<Response<pb::Unit>, Status> {
        self.observed("create_unit", async move {
            let svc = self.svc(&request)?;
            let req = request.into_inner();
            let create = api::CreateUnit { unit_type: req.unit_type, payload: payload_in(req.payload)? };
            Ok(Response::new(unit_out(call(&svc, move |s| s.create_unit(create)).await?)))
        })
        .await
    }

    async fn get_unit(&self, request: Request<pb::UnitRequest>) -> Result<Response<pb::Unit>, Status> {
        self.observed("get_unit", async move {
            let svc = self.svc(&request)?;
            let id = request.into_inner().id;
            Ok(Response::new(unit_out(call(&svc, move |s| s.get_unit(&id)).await?)))
        })
        .await
    }

    async fn list_units(&self, request: Request<pb::ListUnitsRequest>) -> Result<Response<pb::ListUnitsReply>, Status> {
        self.observed("list_units", async move {
            let svc = self.svc(&request)?;
            let req = request.into_inner();
            let list =
                api::ListUnits { unit_type: req.unit_type, limit: req.limit.map(|n| n as usize), after: req.after };
            let list = call(&svc, move |s| s.list_units(list)).await?;
            Ok(Response::new(pb::ListUnitsReply {
                units: list.units.into_iter().map(unit_out).collect(),
                next: list.next,
            }))
        })
        .await
    }

    async fn update_unit(&self, request: Request<pb::UpdateUnitRequest>) -> Result<Response<pb::Unit>, Status> {
        self.observed("update_unit", async move {
            let svc = self.svc(&request)?;
            let req = request.into_inner();
            let update = api::UpdateUnit { payload: payload_in(req.payload)? };
            let unit = call(&svc, move |s| s.update_unit(&req.id, update, req.expected_revision)).await?;
            Ok(Response::new(unit_out(unit)))
        })
        .await
    }

    async fn archive_unit(&self, request: Request<pb::UnitRequest>) -> Result<Response<pb::Unit>, Status> {
        self.observed("archive_unit", async move {
            let svc = self.svc(&request)?;
            let req = request.into_inner();
            let unit = call(&svc, move |s| s.archive_unit(&req.id, req.expected_revision)).await?;
            Ok(Response::new(unit_out(unit)))
        })
        .await
    }

    async fn unarchive_unit(&self, request: Request<pb::UnitRequest>) -> Result<Response<pb::Unit>, Status> {
        self.observed("unarchive_unit", async move {
            let svc = self.svc(&request)?;
            let req = request.into_inner();
            let unit = call(&svc, move |s| s.unarchive_unit(&req.id, req.expected_revision)).await?;
            Ok(Response::new(unit_out(unit)))
        })
        .await
    }

    async fn delete_unit(&self, request: Request<pb::UnitRequest>) -> Result<Response<pb::Empty>, Status> {
        self.observed("delete_unit", async move {
            let svc = self.svc(&request)?;
            let req = request.into_inner();
            call(&svc, move |s| s.delete_unit(&req.id, req.expected_revision)).await?;
            Ok(Response::new(pb::Empty {}))
        })
        .await
    }

    async fn create_relation(
        &self,
        request: Request<pb::CreateRelationRequest>,
    ) -> Result<Response<pb::Relation>, Status> {
        self.observed("create_relation", async move {
            let svc = self.svc(&request)?;
            let req = request.into_inner();
            let create = api::CreateRelation { from: req.from, to: req.to, relation_type: req.relation_type };
            Ok(Response::new(relation_out(call(&svc, move |s| s.create_relation(create)).await?)))
        })
        .await
    }

    async fn list_relations(
        &self,
        request: Request<pb::ListRelationsRequest>,
    ) -> Result<Response<pb::ListRelationsReply>, Status> {
        self.observed("list_relations", async move {
            let svc = self.svc(&request)?;
            let req = request.into_inner();
            let list =
                api::ListRelations { direction: direction_in(req.direction()), relation_type: req.relation_type };
            let list = call(&svc, move |s| s.list_relations(&req.unit, list)).await?;
            Ok(Response::new(pb::ListRelationsReply {
                relations: list.relations.into_iter().map(relation_out).collect(),
            }))
        })
        .await
    }

    async fn delete_relation(&self, request: Request<pb::RelationRequest>) -> Result<Response<pb::Empty>, Status> {
        self.observed("delete_relation", async move {
            let svc = self.svc(&request)?;
            let id = request.into_inner().id;
            call(&svc, move |s| s.delete_relation(&id)).await?;
            Ok(Response::new(pb::Empty {}))
        })
        .await
    }

    async fn define_schema(&self, request: Request<pb::Schema>) -> Result<Response<pb::Schema>, Status> {
        self.observed("define_schema", async move {
            let svc = self.svc(&request)?;
            let schema = schema_in(request.into_inner())?;
            Ok(Response::new(schema_out(call(&svc, move |s| s.define_schema(schema)).await?)))
        })
        .await
    }

    async fn get_schema(&self, request: Request<pb::SchemaRequest>) -> Result<Response<pb::Schema>, Status> {
        self.observed("get_schema", async move {
            let svc = self.svc(&request)?;
            let unit_type = request.into_inner().unit_type;
            Ok(Response::new(schema_out(call(&svc, move |s| s.get_schema(&unit_type)).await?)))
        })
        .await
    }

    async fn query(&self, request: Request<pb::QueryRequest>) -> Result<Response<pb::QueryReply>, Status> {
        self.observed("query", async move {
            let svc = self.svc(&request)?;
            let req = api::QueryRequest { query: request.into_inner().query };
            let reply = call(&svc, move |s| s.query(req)).await?;
            Ok(Response::new(pb::QueryReply {
                aliases: reply.aliases,
                rows: reply
                    .rows
                    .into_iter()
                    .map(|row| pb::Row { units: row.into_iter().map(unit_out).collect() })
                    .collect(),
                access: Some(access_out(reply.access)),
            }))
        })
        .await
    }

    async fn explain(&self, request: Request<pb::QueryRequest>) -> Result<Response<pb::ExplainReply>, Status> {
        self.observed("explain", async move {
            let svc = self.svc(&request)?;
            let req = api::QueryRequest { query: request.into_inner().query };
            let plan = call(&svc, move |s| s.explain(req)).await?;
            Ok(Response::new(pb::ExplainReply {
                root_alias: plan.root_alias,
                root_type: plan.root_type,
                candidates: plan
                    .candidates
                    .into_iter()
                    .map(|c| pb::AccessCandidate { access: Some(access_out(c.access)), cost: c.cost })
                    .collect(),
                chosen: u32::try_from(plan.chosen).unwrap_or(u32::MAX),
                joins: plan
                    .joins
                    .into_iter()
                    .map(|j| pb::JoinStep {
                        alias: j.alias,
                        unit_type: j.unit_type,
                        relation_type: j.relation_type,
                        direction: direction_out(j.direction).into(),
                        from: j.from,
                    })
                    .collect(),
            }))
        })
        .await
    }

    async fn search(&self, request: Request<pb::SearchRequest>) -> Result<Response<pb::SearchReply>, Status> {
        self.observed("search", async move {
            let svc = self.svc(&request)?;
            let search = search_in(request.into_inner());
            let reply = call(&svc, move |s| s.search(search)).await?;
            Ok(Response::new(pb::SearchReply {
                hits: reply
                    .hits
                    .into_iter()
                    .map(|h| pb::Hit {
                        id: h.id,
                        score: h.score,
                        ranks: Some(pb::Ranks {
                            keyword: h.ranks.keyword,
                            vector: h.ranks.vector,
                            graph: h.ranks.graph,
                        }),
                        unit: Some(unit_out(h.unit)),
                    })
                    .collect(),
            }))
        })
        .await
    }
}

// ------------------------------------------------------------ conversions in

fn value_in(value: pb::Value) -> Res<FieldValue> {
    use pb::value::Kind;
    match value.kind {
        Some(Kind::BoolValue(b)) => Ok(FieldValue::Bool(b)),
        Some(Kind::IntValue(i)) => Ok(FieldValue::Int(i)),
        Some(Kind::FloatValue(f)) => Ok(FieldValue::Float(f)),
        Some(Kind::TextValue(t)) => Ok(FieldValue::Text(t)),
        Some(Kind::BytesValue(b)) => Ok(FieldValue::Bytes(b)),
        None => Err(ApiError::invalid(
            "a field value has none of bool_value, int_value, float_value, text_value, bytes_value set",
        )),
    }
}

fn payload_in(payload: Option<pb::Payload>) -> Res<api::Payload> {
    use pb::payload::Kind;
    match payload.and_then(|p| p.kind) {
        Some(Kind::Text(text)) => Ok(api::Payload::Text(text)),
        Some(Kind::Bytes(bytes)) => Ok(api::Payload::Hex(Hex(bytes))),
        Some(Kind::Fields(fields)) => {
            let fields = fields
                .values
                .into_iter()
                .map(|(name, v)| Ok((name, Field(value_in(v)?))))
                .collect::<Res<BTreeMap<_, _>>>()?;
            Ok(api::Payload::Fields(fields))
        }
        None => Err(ApiError::invalid("the payload sets none of text, bytes, fields")),
    }
}

fn direction_in(d: pb::Direction) -> api::Direction {
    match d {
        pb::Direction::Out => api::Direction::Out,
        pb::Direction::In => api::Direction::In,
        pb::Direction::Both | pb::Direction::Unspecified => api::Direction::Both,
    }
}

fn field_type_in(t: pb::FieldType) -> Res<api::FieldType> {
    match t {
        pb::FieldType::Bool => Ok(api::FieldType::Bool),
        pb::FieldType::Int => Ok(api::FieldType::Int),
        pb::FieldType::Float => Ok(api::FieldType::Float),
        pb::FieldType::Text => Ok(api::FieldType::Text),
        pb::FieldType::Bytes => Ok(api::FieldType::Bytes),
        pb::FieldType::Unspecified => Err(ApiError::invalid("a field has no type")),
    }
}

fn schema_in(schema: pb::Schema) -> Res<api::Schema> {
    let fields = schema
        .fields
        .into_iter()
        .map(|f| {
            Ok(api::FieldDef {
                field_type: field_type_in(f.field_type())?,
                name: f.name,
                required: f.required,
                unique: f.unique,
                indexed: f.indexed,
            })
        })
        .collect::<Res<_>>()?;
    Ok(api::Schema { unit_type: schema.unit_type, fields })
}

fn search_in(req: pb::SearchRequest) -> api::SearchRequest {
    api::SearchRequest {
        text: req.text,
        vector: req.vector.map(|v| api::VectorLeg {
            metric: match v.metric() {
                pb::Metric::Cosine => api::Metric::Cosine,
                pb::Metric::Dot => api::Metric::Dot,
            },
            query: v.query,
            vectors: v.vectors.into_iter().map(|(id, vector)| (id, vector.values)).collect(),
        }),
        graph: req.graph.map(|g| api::GraphLeg {
            direction: direction_in(g.direction()),
            seeds: g.seeds,
            relation_types: (!g.relation_types.is_empty()).then_some(g.relation_types),
            max_depth: g.max_depth,
            limit: g.limit.map(|n| n as usize),
            visit_budget: g.visit_budget,
        }),
        weights: req.weights.map(|w| api::Weights { keyword: w.keyword, vector: w.vector, graph: w.graph }),
        rrf_k: req.rrf_k,
        candidates: req.candidates.map(|n| n as usize),
        limit: req.limit.map(|n| n as usize),
    }
}

// ----------------------------------------------------------- conversions out

fn value_out(value: FieldValue) -> pb::Value {
    use pb::value::Kind;
    let kind = match value {
        FieldValue::Bool(b) => Kind::BoolValue(b),
        FieldValue::Int(i) => Kind::IntValue(i),
        FieldValue::Float(f) => Kind::FloatValue(f),
        FieldValue::Text(t) => Kind::TextValue(t),
        FieldValue::Bytes(b) => Kind::BytesValue(b),
    };
    pb::Value { kind: Some(kind) }
}

fn payload_out(payload: api::Payload) -> pb::Payload {
    use pb::payload::Kind;
    let kind = match payload {
        api::Payload::Text(text) => Kind::Text(text),
        api::Payload::Hex(Hex(bytes)) => Kind::Bytes(bytes),
        api::Payload::Fields(fields) => {
            let values: HashMap<String, pb::Value> =
                fields.into_iter().map(|(k, Field(v))| (k, value_out(v))).collect();
            Kind::Fields(pb::Fields { values })
        }
    };
    pb::Payload { kind: Some(kind) }
}

fn unit_out(unit: api::UnitView) -> pb::Unit {
    pb::Unit {
        id: unit.id,
        unit_type: unit.unit_type,
        lifecycle: unit.lifecycle.to_owned(),
        revision: unit.revision,
        created_at_us: unit.created_at_us,
        updated_at_us: unit.updated_at_us,
        payload: unit.payload.map(payload_out),
    }
}

fn direction_out(d: api::Direction) -> pb::Direction {
    match d {
        api::Direction::Out => pb::Direction::Out,
        api::Direction::In => pb::Direction::In,
        api::Direction::Both => pb::Direction::Both,
    }
}

fn relation_out(r: api::RelationView) -> pb::Relation {
    pb::Relation {
        id: r.id,
        from: r.from,
        to: r.to,
        relation_type: r.relation_type,
        provenance: r.provenance.to_owned(),
        direction: r.direction.map(|d| direction_out(d).into()),
    }
}

fn schema_out(schema: api::Schema) -> pb::Schema {
    pb::Schema {
        unit_type: schema.unit_type,
        fields: schema
            .fields
            .into_iter()
            .map(|f| pb::FieldDef {
                name: f.name,
                field_type: match f.field_type {
                    api::FieldType::Bool => pb::FieldType::Bool,
                    api::FieldType::Int => pb::FieldType::Int,
                    api::FieldType::Float => pb::FieldType::Float,
                    api::FieldType::Text => pb::FieldType::Text,
                    api::FieldType::Bytes => pb::FieldType::Bytes,
                }
                .into(),
                required: f.required,
                unique: f.unique,
                indexed: f.indexed,
            })
            .collect(),
    }
}

fn access_out(access: api::Access) -> pb::Access {
    use pb::access::Kind;
    pb::Access {
        kind: Some(match access {
            api::Access::TypeScan => Kind::TypeScan(pb::Empty {}),
            api::Access::FieldIndex { field } => Kind::FieldIndex(field),
        }),
    }
}

#[cfg(test)]
mod tests {
    use tonic::Code;

    use super::*;

    #[test]
    fn every_kind_maps_to_its_grpc_code() {
        let table = [
            (Kind::InvalidRequest, Code::InvalidArgument),
            (Kind::SchemaViolation, Code::InvalidArgument),
            (Kind::Unauthenticated, Code::Unauthenticated),
            (Kind::NotFound, Code::NotFound),
            (Kind::MethodNotAllowed, Code::Unimplemented),
            (Kind::RevisionConflict, Code::Aborted),
            (Kind::Conflict, Code::FailedPrecondition),
            (Kind::PayloadTooLarge, Code::ResourceExhausted),
            (Kind::Unavailable, Code::Unavailable),
            (Kind::Internal, Code::Internal),
        ];
        for (kind, code) in table {
            let status = Status::from(ApiError::new(kind, "why"));
            assert_eq!(status.code(), code, "{kind:?}");
            assert_eq!(status.message(), "why");
            assert_eq!(status.metadata().get(ERROR_CODE_KEY).unwrap(), kind.code());
        }
    }

    #[test]
    fn a_payload_must_set_exactly_one_member() {
        assert!(payload_in(None).is_err());
        assert!(payload_in(Some(pb::Payload { kind: None })).is_err());
        let text = payload_in(Some(pb::Payload { kind: Some(pb::payload::Kind::Text("x".into())) })).unwrap();
        assert_eq!(text, api::Payload::Text("x".into()));
        let empty_value = pb::Fields { values: HashMap::from([("a".to_owned(), pb::Value { kind: None })]) };
        assert!(payload_in(Some(pb::Payload { kind: Some(pb::payload::Kind::Fields(empty_value)) })).is_err());
    }

    #[test]
    fn an_unspecified_direction_is_both_and_an_unspecified_field_type_is_refused() {
        assert_eq!(direction_in(pb::Direction::Unspecified), api::Direction::Both);
        assert_eq!(direction_in(pb::Direction::Out), api::Direction::Out);
        assert_eq!(direction_in(pb::Direction::In), api::Direction::In);
        assert!(field_type_in(pb::FieldType::Unspecified).is_err());
    }
}
