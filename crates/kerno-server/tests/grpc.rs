//! End to end: every test starts the real `kerno-server` binary with gRPC on and calls it through
//! the generated client.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::collections::HashMap;

use common::{Server, TempDir};
use kerno_server::grpc::{ERROR_CODE_KEY, pb};
use pb::kerno_client::KernoClient;
use serde_json::json;
use tonic::transport::Channel;
use tonic::{Code, Status};

type Client = KernoClient<Channel>;

fn start() -> (TempDir, Server) {
    let dir = TempDir::new();
    let server = Server::start_with(&dir.db(), &["--grpc-listen", "127.0.0.1:0"]);
    (dir, server)
}

async fn client(server: &Server) -> Client {
    KernoClient::connect(format!("http://{}", server.grpc.unwrap())).await.unwrap()
}

fn text(t: &str) -> Option<pb::Payload> {
    Some(pb::Payload { kind: Some(pb::payload::Kind::Text(t.into())) })
}

fn fields(values: Vec<(&str, pb::value::Kind)>) -> Option<pb::Payload> {
    let values: HashMap<String, pb::Value> =
        values.into_iter().map(|(k, v)| (k.to_owned(), pb::Value { kind: Some(v) })).collect();
    Some(pb::Payload { kind: Some(pb::payload::Kind::Fields(pb::Fields { values })) })
}

async fn note(c: &mut Client, body: &str) -> String {
    let unit = c.create_unit(pb::CreateUnitRequest { unit_type: "note".into(), payload: text(body) }).await.unwrap();
    unit.into_inner().id
}

fn unit_ref(id: &str) -> pb::UnitRequest {
    pb::UnitRequest { id: id.to_owned(), expected_revision: None }
}

fn code_of(status: &Status) -> &str {
    status.metadata().get(ERROR_CODE_KEY).unwrap().to_str().unwrap()
}

async fn relate(c: &mut Client, from: &str, to: &str, relation_type: &str) -> String {
    let req = pb::CreateRelationRequest { from: from.into(), to: to.into(), relation_type: relation_type.into() };
    c.create_relation(req).await.unwrap().into_inner().id
}

#[tokio::test]
async fn health_answers_ok() {
    let (_dir, server) = start();
    let health = client(&server).await.health(pb::Empty {}).await.unwrap().into_inner();
    assert_eq!((health.status.as_str(), health.version.as_str()), ("ok", env!("CARGO_PKG_VERSION")));
}

#[tokio::test]
async fn a_unit_goes_through_its_whole_life() {
    let (_dir, server) = start();
    let mut c = client(&server).await;
    let created = c
        .create_unit(pb::CreateUnitRequest { unit_type: "note".into(), payload: text("first") })
        .await
        .unwrap()
        .into_inner();
    assert_eq!((created.unit_type.as_str(), created.lifecycle.as_str(), created.revision), ("note", "active", 1));
    assert_eq!(created.id.len(), 32);
    assert_eq!(created.payload, text("first"));
    assert!(created.created_at_us > 0);
    let id = created.id.clone();
    assert_eq!(c.get_unit(unit_ref(&id)).await.unwrap().into_inner(), created);

    let updated = c
        .update_unit(pb::UpdateUnitRequest { id: id.clone(), payload: text("second"), expected_revision: Some(1) })
        .await
        .unwrap()
        .into_inner();
    assert_eq!((updated.revision, updated.payload), (2, text("second")));

    let archived = c.archive_unit(unit_ref(&id)).await.unwrap().into_inner();
    assert_eq!((archived.lifecycle.as_str(), archived.revision), ("archived", 3));
    let listed =
        c.list_units(pb::ListUnitsRequest { unit_type: Some("note".into()), ..Default::default() }).await.unwrap();
    assert!(listed.into_inner().units.is_empty(), "an archived unit is not listed");
    let back = c.unarchive_unit(unit_ref(&id)).await.unwrap().into_inner();
    assert_eq!(back.lifecycle, "active");

    c.delete_unit(unit_ref(&id)).await.unwrap();
    let tombstone = c.get_unit(unit_ref(&id)).await.unwrap().into_inner();
    assert_eq!((tombstone.lifecycle.as_str(), tombstone.payload), ("deleted", None));
}

#[tokio::test]
async fn expected_revision_guards_every_change() {
    let (_dir, server) = start();
    let mut c = client(&server).await;
    let id = note(&mut c, "v1").await;
    let at = |revision: u64| pb::UnitRequest { id: id.clone(), expected_revision: Some(revision) };
    for stale in [
        c.archive_unit(at(7)).await.unwrap_err(),
        c.unarchive_unit(at(7)).await.unwrap_err(),
        c.delete_unit(at(7)).await.unwrap_err(),
    ] {
        assert_eq!((stale.code(), code_of(&stale)), (Code::Aborted, "revision_conflict"));
    }
    assert_eq!(c.get_unit(unit_ref(&id)).await.unwrap().into_inner().revision, 1, "refused changes change nothing");
    assert_eq!(c.archive_unit(at(1)).await.unwrap().into_inner().revision, 2);
    assert_eq!(c.unarchive_unit(at(2)).await.unwrap().into_inner().revision, 3);
    c.delete_unit(at(3)).await.unwrap();
}

#[tokio::test]
async fn units_are_listed_in_pages() {
    let (_dir, server) = start();
    let mut c = client(&server).await;
    let mut ids = Vec::new();
    for i in 0..5 {
        ids.push(note(&mut c, &format!("n{i}")).await);
    }
    ids.sort();
    let page = |limit: u32, after: Option<&str>| pb::ListUnitsRequest {
        unit_type: Some("note".into()),
        limit: Some(limit),
        after: after.map(str::to_owned),
    };
    let first = c.list_units(page(2, None)).await.unwrap().into_inner();
    assert_eq!(first.units.iter().map(|u| u.id.clone()).collect::<Vec<_>>(), ids[..2]);
    assert_eq!(first.next.as_deref(), Some(ids[1].as_str()));
    let last = c.list_units(page(2, Some(&ids[3]))).await.unwrap().into_inner();
    assert_eq!(last.units.iter().map(|u| u.id.clone()).collect::<Vec<_>>(), ids[4..]);
    assert_eq!(last.next, None);
    let default = c.list_units(pb::ListUnitsRequest::default()).await.unwrap().into_inner();
    assert_eq!(default.units.len(), 5);
    let too_many = c.list_units(page(1001, None)).await.unwrap_err();
    assert_eq!(too_many.code(), Code::InvalidArgument);
}

#[tokio::test]
async fn errors_carry_a_grpc_code_and_the_machine_readable_one() {
    let (_dir, server) = start();
    let mut c = client(&server).await;
    let id = note(&mut c, "v1").await;
    let missing = c.get_unit(unit_ref(&"00".repeat(16))).await.unwrap_err();
    assert_eq!((missing.code(), code_of(&missing)), (Code::NotFound, "not_found"));
    let bad_id = c.get_unit(unit_ref("xyz")).await.unwrap_err();
    assert_eq!((bad_id.code(), code_of(&bad_id)), (Code::InvalidArgument, "invalid_request"));
    let stale = c
        .update_unit(pb::UpdateUnitRequest { id: id.clone(), payload: text("x"), expected_revision: Some(9) })
        .await
        .unwrap_err();
    assert_eq!((stale.code(), code_of(&stale)), (Code::Aborted, "revision_conflict"));
    let no_payload =
        c.create_unit(pb::CreateUnitRequest { unit_type: "note".into(), payload: None }).await.unwrap_err();
    assert_eq!((no_payload.code(), code_of(&no_payload)), (Code::InvalidArgument, "invalid_request"));
    // A unit with a relation cannot be deleted.
    let other = note(&mut c, "other").await;
    relate(&mut c, &id, &other, "about").await;
    let refused = c.delete_unit(unit_ref(&id)).await.unwrap_err();
    assert_eq!((refused.code(), code_of(&refused)), (Code::FailedPrecondition, "conflict"));
    let refused = c.delete_unit(unit_ref(&other)).await.unwrap_err();
    assert_eq!(refused.code(), Code::FailedPrecondition, "the target of a relation is held too");
}

#[tokio::test]
async fn relations_are_created_listed_from_both_ends_and_deleted() {
    let (_dir, server) = start();
    let mut c = client(&server).await;
    let (a, b, d) = (note(&mut c, "a").await, note(&mut c, "b").await, note(&mut c, "d").await);
    let outgoing = relate(&mut c, &b, &d, "about").await;
    let incoming = relate(&mut c, &a, &b, "about").await;
    let list = |unit: &str, direction: pb::Direction, relation_type: Option<&str>| pb::ListRelationsRequest {
        unit: unit.to_owned(),
        direction: direction.into(),
        relation_type: relation_type.map(str::to_owned),
    };
    let ids = |reply: pb::ListRelationsReply| {
        reply.relations.into_iter().map(|r| (r.direction(), r.id)).map(|(d, id)| (id, d)).collect::<Vec<_>>()
    };

    let out = c.list_relations(list(&b, pb::Direction::Out, None)).await.unwrap().into_inner();
    assert_eq!(ids(out), [(outgoing.clone(), pb::Direction::Out)]);
    let inc = c.list_relations(list(&b, pb::Direction::In, None)).await.unwrap().into_inner();
    assert_eq!(ids(inc), [(incoming.clone(), pb::Direction::In)]);
    // Unspecified means both.
    let both = c.list_relations(list(&b, pb::Direction::Unspecified, None)).await.unwrap().into_inner();
    assert_eq!(ids(both), [(outgoing.clone(), pb::Direction::Out), (incoming.clone(), pb::Direction::In)]);
    let none = c.list_relations(list(&b, pb::Direction::Both, Some("other"))).await.unwrap().into_inner();
    assert!(none.relations.is_empty());

    let first = c.list_relations(list(&a, pb::Direction::Out, None)).await.unwrap().into_inner().relations.remove(0);
    assert_eq!(
        (first.from.as_str(), first.to.as_str(), first.relation_type.as_str()),
        (a.as_str(), b.as_str(), "about")
    );
    assert_eq!(first.provenance, "declared");

    c.delete_relation(pb::RelationRequest { id: outgoing.clone() }).await.unwrap();
    let after = c.list_relations(list(&d, pb::Direction::Both, None)).await.unwrap().into_inner();
    assert!(after.relations.is_empty());
    let gone = c.delete_relation(pb::RelationRequest { id: outgoing }).await.unwrap_err();
    assert_eq!(gone.code(), Code::NotFound);
    let unknown_unit = c.list_relations(list(&"00".repeat(16), pb::Direction::Both, None)).await.unwrap_err();
    assert_eq!(unknown_unit.code(), Code::NotFound);
}

fn user_schema() -> pb::Schema {
    let field = |name: &str, field_type: pb::FieldType, unique: bool, indexed: bool| pb::FieldDef {
        name: name.into(),
        field_type: field_type.into(),
        required: unique,
        unique,
        indexed,
    };
    pb::Schema {
        unit_type: "user".into(),
        fields: vec![
            field("email", pb::FieldType::Text, true, false),
            field("age", pb::FieldType::Int, false, true),
            field("score", pb::FieldType::Float, false, false),
            field("avatar", pb::FieldType::Bytes, false, false),
            field("admin", pb::FieldType::Bool, false, false),
        ],
    }
}

fn user(email: &str, age: i64) -> pb::CreateUnitRequest {
    use pb::value::Kind;
    pb::CreateUnitRequest {
        unit_type: "user".into(),
        payload: fields(vec![("email", Kind::TextValue(email.into())), ("age", Kind::IntValue(age))]),
    }
}

#[tokio::test]
async fn a_schema_is_defined_and_typed_units_hold_every_field_type() {
    use pb::value::Kind;
    let (_dir, server) = start();
    let mut c = client(&server).await;
    let defined = c.define_schema(user_schema()).await.unwrap().into_inner();
    let email = defined.fields.iter().find(|f| f.name == "email").unwrap();
    assert!(email.required && email.unique && email.indexed, "unique implies indexed");
    // The types of the answer are the types that were asked for, field by field (the answer to
    // `get_schema` is built by the same code, so comparing the two would prove nothing).
    let types: Vec<(&str, pb::FieldType)> = defined.fields.iter().map(|f| (f.name.as_str(), f.field_type())).collect();
    assert_eq!(
        types,
        [
            ("admin", pb::FieldType::Bool),
            ("age", pb::FieldType::Int),
            ("avatar", pb::FieldType::Bytes),
            ("email", pb::FieldType::Text),
            ("score", pb::FieldType::Float),
        ]
    );
    assert_eq!(c.get_schema(pb::SchemaRequest { unit_type: "user".into() }).await.unwrap().into_inner(), defined);
    assert_eq!(c.get_schema(pb::SchemaRequest { unit_type: "none".into() }).await.unwrap_err().code(), Code::NotFound);
    assert_eq!(c.define_schema(user_schema()).await.unwrap_err().code(), Code::InvalidArgument);
    let untyped =
        pb::Schema { unit_type: "t".into(), fields: vec![pb::FieldDef { name: "x".into(), ..Default::default() }] };
    assert_eq!(c.define_schema(untyped).await.unwrap_err().code(), Code::InvalidArgument);

    let all = vec![
        ("email", Kind::TextValue("ada@x".into())),
        ("age", Kind::IntValue(36)),
        ("score", Kind::FloatValue(4.5)),
        ("avatar", Kind::BytesValue(vec![0, 255])),
        ("admin", Kind::BoolValue(true)),
    ];
    let created =
        c.create_unit(pb::CreateUnitRequest { unit_type: "user".into(), payload: fields(all.clone()) }).await.unwrap();
    assert_eq!(created.into_inner().payload, fields(all));

    let dup = c.create_unit(user("ada@x", 1)).await.unwrap_err();
    assert_eq!((dup.code(), code_of(&dup)), (Code::InvalidArgument, "schema_violation"));
    let wrong = c
        .create_unit(pb::CreateUnitRequest {
            unit_type: "user".into(),
            payload: fields(vec![("email", Kind::IntValue(1))]),
        })
        .await
        .unwrap_err();
    assert_eq!(code_of(&wrong), "schema_violation");
}

#[tokio::test]
async fn the_query_language_runs_and_explains() {
    let (_dir, server) = start();
    let mut c = client(&server).await;
    c.define_schema(user_schema()).await.unwrap();
    let team_schema = pb::Schema {
        unit_type: "team".into(),
        fields: vec![pb::FieldDef {
            name: "name".into(),
            field_type: pb::FieldType::Text.into(),
            ..Default::default()
        }],
    };
    c.define_schema(team_schema).await.unwrap();
    let team = c
        .create_unit(pb::CreateUnitRequest {
            unit_type: "team".into(),
            payload: fields(vec![("name", pb::value::Kind::TextValue("core".into()))]),
        })
        .await
        .unwrap()
        .into_inner()
        .id;
    for i in 0..8 {
        let unit = c.create_unit(user(&format!("u{i}@x"), 20 + i * 3)).await.unwrap().into_inner();
        if i % 2 == 0 {
            relate(&mut c, &unit.id, &team, "member_of").await;
        }
    }
    let ask = |q: &str| pb::QueryRequest { query: q.to_owned() };

    let rows = c.query(ask("FIND user AS u WHERE u.age >= 35 ORDER BY u.age")).await.unwrap().into_inner();
    assert_eq!(rows.aliases, ["u"]);
    let ages: Vec<i64> = rows
        .rows
        .iter()
        .map(|r| match &r.units[0].payload.as_ref().unwrap().kind {
            Some(pb::payload::Kind::Fields(f)) => match f.values["age"].kind {
                Some(pb::value::Kind::IntValue(i)) => i,
                _ => panic!("age is not an int"),
            },
            _ => panic!("not fields"),
        })
        .collect();
    assert_eq!(ages, [35, 38, 41]);
    assert_eq!(rows.access.unwrap().kind, Some(pb::access::Kind::FieldIndex("age".into())));

    let joined = c.query(ask("FIND user AS u JOIN team AS t VIA member_of")).await.unwrap().into_inner();
    assert_eq!((joined.aliases.clone(), joined.rows.len()), (vec!["u".to_owned(), "t".to_owned()], 4));
    assert!(joined.rows.iter().all(|r| r.units[1].id == team));
    assert_eq!(joined.access.unwrap().kind, Some(pb::access::Kind::TypeScan(pb::Empty {})));

    let plan =
        c.explain(ask("FIND user AS u JOIN team AS t VIA member_of WHERE u.age = 29")).await.unwrap().into_inner();
    assert_eq!((plan.root_alias.as_str(), plan.root_type.as_str(), plan.chosen), ("u", "user", 1));
    assert_eq!(plan.candidates[0].access.as_ref().unwrap().kind, Some(pb::access::Kind::TypeScan(pb::Empty {})));
    assert_eq!(plan.candidates[1].access.as_ref().unwrap().kind, Some(pb::access::Kind::FieldIndex("age".into())));
    assert!(plan.candidates[1].cost < plan.candidates[0].cost);
    assert_eq!(plan.joins.len(), 1);
    let join = &plan.joins[0];
    assert_eq!(
        (join.alias.as_str(), join.unit_type.as_str(), join.relation_type.as_str(), join.from.as_str()),
        ("t", "team", "member_of", "u")
    );
    assert_eq!(join.direction(), pb::Direction::Out);

    for bad in ["FIND", "FIND user WHERE nope = 1"] {
        assert_eq!(c.query(ask(bad)).await.unwrap_err().code(), Code::InvalidArgument, "{bad}");
        assert_eq!(c.explain(ask(bad)).await.unwrap_err().code(), Code::InvalidArgument, "{bad}");
    }
}

fn earns(weight: u64, rrf_k: u64, rank: u64) -> u64 {
    weight * 1_000_000_000 / (rrf_k + rank)
}

#[tokio::test]
async fn hybrid_search_fuses_its_legs() {
    let (_dir, server) = start();
    let mut c = client(&server).await;
    let both = note(&mut c, "rust memory").await;
    let keyword_only = note(&mut c, "rust").await;
    let seed = note(&mut c, "seed").await;
    relate(&mut c, &seed, &both, "about").await;

    // Keyword leg alone: defaults are weight 1, k 60, ten hits.
    let reply =
        c.search(pb::SearchRequest { text: Some("rust".into()), ..Default::default() }).await.unwrap().into_inner();
    assert_eq!(reply.hits.len(), 2);
    let top = &reply.hits[0];
    assert_eq!(top.ranks.as_ref().unwrap(), &pb::Ranks { keyword: Some(1), vector: None, graph: None });
    assert_eq!(top.score, earns(1, 60, 1));
    assert_eq!(top.unit.as_ref().unwrap().id, top.id);

    // Keyword and graph legs: the unit both find outranks the one only the keyword leg finds.
    let graph = pb::GraphLeg { seeds: vec![seed.clone()], ..Default::default() };
    let fused = c
        .search(pb::SearchRequest { text: Some("rust".into()), graph: Some(graph.clone()), ..Default::default() })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(fused.hits[0].id, both);
    let ranks = fused.hits[0].ranks.unwrap();
    assert_eq!(
        fused.hits[0].score,
        earns(1, 60, ranks.keyword.unwrap().into()) + earns(1, 60, ranks.graph.unwrap().into())
    );
    assert!(fused.hits.iter().any(|h| h.id == keyword_only));
    assert!(fused.hits.iter().all(|h| h.id != seed), "a seed is not a hit");

    // Weights: 0 switches a leg off, and an absent weight is 1.
    let off = pb::Weights { keyword: Some(0), ..Default::default() };
    let graph_only = c
        .search(pb::SearchRequest {
            text: Some("rust".into()),
            graph: Some(graph.clone()),
            weights: Some(off),
            ..Default::default()
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(graph_only.hits.iter().map(|h| h.id.clone()).collect::<Vec<_>>(), std::slice::from_ref(&both));
    let heavy = pb::Weights { graph: Some(3), ..Default::default() };
    let weighted = c
        .search(pb::SearchRequest {
            graph: Some(graph),
            weights: Some(heavy),
            rrf_k: Some(0),
            limit: Some(1),
            ..Default::default()
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(weighted.hits[0].score, earns(3, 0, 1));

    // Vector leg: the request carries the vectors.
    let vector = |q: Vec<f32>, metric: pb::Metric, vectors: Vec<(&str, Vec<f32>)>| pb::VectorLeg {
        query: q,
        metric: metric.into(),
        vectors: vectors.into_iter().map(|(id, values)| (id.to_owned(), pb::Vector { values })).collect(),
    };
    let leg =
        vector(vec![1.0, 0.0], pb::Metric::Cosine, vec![(&both, vec![5.0, 5.0]), (&keyword_only, vec![0.9, 0.0])]);
    let by_cosine = c.search(pb::SearchRequest { vector: Some(leg), ..Default::default() }).await.unwrap().into_inner();
    assert_eq!(by_cosine.hits.iter().map(|h| h.id.clone()).collect::<Vec<_>>(), [keyword_only.clone(), both.clone()]);
    assert_eq!(by_cosine.hits[0].ranks.as_ref().unwrap().vector, Some(1));
    let leg = vector(vec![1.0, 0.0], pb::Metric::Dot, vec![(&both, vec![5.0, 5.0]), (&keyword_only, vec![0.9, 0.0])]);
    let by_dot = c.search(pb::SearchRequest { vector: Some(leg), ..Default::default() }).await.unwrap().into_inner();
    assert_eq!(by_dot.hits.iter().map(|h| h.id.clone()).collect::<Vec<_>>(), [both, keyword_only]);

    for bad in [
        pb::SearchRequest::default(),
        pb::SearchRequest { text: Some("x".into()), limit: Some(0), ..Default::default() },
    ] {
        assert_eq!(c.search(bad).await.unwrap_err().code(), Code::InvalidArgument);
    }
}

#[tokio::test]
async fn every_parameter_of_a_search_reaches_the_core() {
    let (_dir, server) = start();
    let mut c = client(&server).await;
    let (a, b, d, e) =
        (note(&mut c, "a").await, note(&mut c, "b").await, note(&mut c, "d").await, note(&mut c, "e").await);
    relate(&mut c, &a, &b, "next").await;
    relate(&mut c, &b, &d, "next").await;
    relate(&mut c, &a, &e, "other").await;
    let ids = |reply: pb::SearchReply| reply.hits.into_iter().map(|h| h.id).collect::<Vec<_>>();
    let graph =
        |seeds: &[&str]| pb::GraphLeg { seeds: seeds.iter().map(|s| (*s).to_owned()).collect(), ..Default::default() };
    let run = |graph: pb::GraphLeg| pb::SearchRequest { graph: Some(graph), ..Default::default() };

    // Two hops by default; one when `max_depth` says so.
    let mut deep = ids(c.search(run(graph(&[&a]))).await.unwrap().into_inner());
    deep.sort();
    let mut expected = vec![b.clone(), d.clone(), e.clone()];
    expected.sort();
    assert_eq!(deep, expected);
    let shallow = pb::GraphLeg { max_depth: Some(1), ..graph(&[&a]) };
    let mut one_hop = ids(c.search(run(shallow)).await.unwrap().into_inner());
    one_hop.sort();
    let mut expected = vec![b.clone(), e.clone()];
    expected.sort();
    assert_eq!(one_hop, expected);
    // Relation types, direction, the graph leg's own limit and its visit budget.
    let only_next =
        pb::GraphLeg { relation_types: vec!["next".into()], direction: pb::Direction::Out.into(), ..graph(&[&a]) };
    assert_eq!(ids(c.search(run(only_next)).await.unwrap().into_inner()), [b.clone(), d.clone()]);
    let incoming = pb::GraphLeg { direction: pb::Direction::In.into(), ..graph(&[&d]) };
    assert_eq!(ids(c.search(run(incoming)).await.unwrap().into_inner()), [b.clone(), a.clone()]);
    let one = pb::GraphLeg { limit: Some(1), ..graph(&[&a]) };
    assert_eq!(c.search(run(one)).await.unwrap().into_inner().hits.len(), 1);
    let starved = pb::GraphLeg { visit_budget: Some(1), ..graph(&[&a]) };
    assert!(c.search(run(starved)).await.unwrap().into_inner().hits.len() < 3);

    // Candidates, and a weight on every leg.
    for i in 0..4 {
        note(&mut c, &format!("bulk {i}")).await;
    }
    let bulk = |candidates: Option<u32>| pb::SearchRequest {
        text: Some("bulk".into()),
        candidates,
        limit: Some(100),
        ..Default::default()
    };
    assert_eq!(c.search(bulk(None)).await.unwrap().into_inner().hits.len(), 4);
    assert_eq!(c.search(bulk(Some(2))).await.unwrap().into_inner().hits.len(), 2);
    let vector = pb::VectorLeg {
        query: vec![1.0, 0.0],
        metric: pb::Metric::Cosine.into(),
        vectors: HashMap::from([(a.clone(), pb::Vector { values: vec![1.0, 0.0] })]),
    };
    let weighted = |keyword, vector_weight, graph_weight| pb::SearchRequest {
        text: Some("a".into()),
        vector: Some(vector.clone()),
        graph: Some(graph(&[&d])),
        weights: Some(pb::Weights { keyword, vector: vector_weight, graph: graph_weight }),
        ..Default::default()
    };
    let score_of = |reply: pb::SearchReply, id: &str| reply.hits.into_iter().find(|h| h.id == id).unwrap().score;
    // `a` is the keyword hit (rank 1), the vector hit (rank 1) and, from `d`, a graph hit (rank 2).
    let base = score_of(c.search(weighted(None, None, None)).await.unwrap().into_inner(), &a);
    assert_eq!(base, earns(1, 60, 1) + earns(1, 60, 1) + earns(1, 60, 2));
    let heavy_keyword = score_of(c.search(weighted(Some(4), None, None)).await.unwrap().into_inner(), &a);
    assert_eq!(heavy_keyword, earns(4, 60, 1) + earns(1, 60, 1) + earns(1, 60, 2));
    let heavy_vector = score_of(c.search(weighted(None, Some(4), None)).await.unwrap().into_inner(), &a);
    assert_eq!(heavy_vector, earns(1, 60, 1) + earns(4, 60, 1) + earns(1, 60, 2));
    let heavy_graph = score_of(c.search(weighted(None, None, Some(4))).await.unwrap().into_inner(), &a);
    assert_eq!(heavy_graph, earns(1, 60, 1) + earns(1, 60, 1) + earns(4, 60, 2));
}

#[tokio::test]
async fn the_two_transports_serve_one_database() {
    let (_dir, server) = start();
    let mut c = client(&server).await;
    let over_http = server.post("/v1/units", &json!({ "type": "note", "payload": { "text": "from http" } })).json();
    let id = over_http["id"].as_str().unwrap();
    let read = c.get_unit(unit_ref(id)).await.unwrap().into_inner();
    assert_eq!(read.payload, text("from http"));
    assert_eq!((read.created_at_us, read.revision), (over_http["created_at_us"].as_i64().unwrap(), 1));

    let over_grpc = note(&mut c, "from grpc").await;
    let read = server.get(&format!("/v1/units/{over_grpc}"));
    assert_eq!(read.json()["payload"], json!({ "text": "from grpc" }));
}

/// Stops `server` the way a service manager does, off the runtime's thread so that a client of
/// this test keeps being polled while the server shuts down.
async fn stop(server: Server) -> std::process::ExitStatus {
    tokio::task::spawn_blocking(move || server.stop()).await.unwrap()
}

#[tokio::test]
async fn a_graceful_stop_ends_both_transports_and_frees_the_directory() {
    let dir = TempDir::new();
    let server = Server::start_with(&dir.db(), &["--grpc-listen", "127.0.0.1:0"]);
    // The client stays connected, idle, while the server is asked to stop.
    let mut c = client(&server).await;
    let id = note(&mut c, "durable").await;
    let started = std::time::Instant::now();
    let status = stop(server).await;
    assert!(status.success(), "{status}");
    assert!(
        started.elapsed() < std::time::Duration::from_secs(8),
        "an idle connection held the shutdown for {:?}",
        started.elapsed()
    );
    drop(c);

    let again = Server::start_with(&dir.db(), &["--grpc-listen", "127.0.0.1:0"]);
    let read = client(&again).await.get_unit(unit_ref(&id)).await.unwrap().into_inner();
    assert_eq!(read.payload, text("durable"));
}

#[test]
fn grpc_is_off_unless_an_address_is_given() {
    let dir = TempDir::new();
    let server = Server::start(&dir.db());
    assert_eq!(server.grpc, None);
}

#[test]
fn an_address_in_use_is_refused_with_its_name() {
    let taken = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let dir = TempDir::new();
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_kerno-server"))
        .arg("--db")
        .arg(dir.db())
        .args(["--listen", "127.0.0.1:0", "--grpc-listen", &taken.local_addr().unwrap().to_string()])
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("cannot listen on"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}
