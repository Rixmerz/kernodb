use super::*;

fn parse(text: &str) -> Query {
    Query::parse(text).unwrap_or_else(|e| panic!("{text:?}: {e}"))
}

fn fails(text: &str) -> ParseError {
    Query::parse(text).expect_err(text)
}

#[test]
fn a_query_of_every_clause_parses_into_its_parts() {
    let q = parse(
        "find user as u join team as t via member_of join \"depends-on\" AS d via \"depends-on\" in from u \
         where u.age >= 30 and (t.name = \"core\" or not u.score < 4.5) \
         order by u.age desc, email limit 10 offset 5",
    );
    assert_eq!(q.root, Binding { unit_type: "user".into(), alias: "u".into() });
    assert_eq!(q.joins.len(), 2);
    assert_eq!((q.joins[0].from.as_str(), q.joins[0].direction), ("u", Direction::Out));
    assert_eq!((q.joins[1].from.as_str(), q.joins[1].direction), ("u", Direction::In));
    assert_eq!(q.joins[1].relation_type, "depends-on");
    assert_eq!(q.order.len(), 2);
    assert!(q.order[0].descending && !q.order[1].descending);
    // A bare field is a field of the first binding.
    assert_eq!(q.order[1].field, FieldRef { alias: "u".into(), field: "email".into() });
    assert_eq!((q.limit, q.offset), (Some(10), 5));
    let Some(Expr::And(terms)) = &q.filter else { panic!("{:?}", q.filter) };
    assert!(matches!(&terms[1], Expr::Or(v) if matches!(v[1], Expr::Not(_))));
}

#[test]
fn the_alias_defaults_to_the_type_and_a_join_starts_from_the_previous_binding() {
    let q = parse("FIND a JOIN b VIA r JOIN c VIA s");
    assert_eq!(q.root.alias, "a");
    assert_eq!((q.joins[0].from.as_str(), q.joins[1].from.as_str()), ("a", "b"));
    assert_eq!((q.limit, q.offset, q.filter), (None, 0, None));
}

#[test]
fn and_binds_tighter_than_or_and_not_tighter_than_both() {
    let q = parse("FIND a WHERE x = 1 OR y = 2 AND NOT z = 3");
    let Some(Expr::Or(v)) = q.filter else { panic!("no OR at the top") };
    assert!(matches!(&v[1], Expr::And(t) if matches!(t[1], Expr::Not(_))));
}

#[test]
fn every_literal_type_and_operator_is_read() {
    let q = parse(
        "FIND a WHERE i = -7 AND f > 1.5e3 AND g <= -0.25 AND t != \"a\\\"b\\\\\\n\" AND b >= x\"00ff\" AND k < true \
         AND n IS NULL AND m IS NOT NULL AND j = 9223372036854775807 AND l = -9223372036854775808",
    );
    let Some(Expr::And(t)) = q.filter else { panic!("not a conjunction") };
    let lits: Vec<_> = t
        .iter()
        .filter_map(|e| match e {
            Expr::Cmp { op, value, .. } => Some((*op, value.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(lits[0], (CmpOp::Eq, FieldValue::Int(-7)));
    assert_eq!(lits[1], (CmpOp::Gt, FieldValue::Float(1500.0)));
    assert_eq!(lits[2], (CmpOp::Le, FieldValue::Float(-0.25)));
    assert_eq!(lits[3], (CmpOp::Ne, FieldValue::Text("a\"b\\\n".into())));
    assert_eq!(lits[4], (CmpOp::Ge, FieldValue::Bytes(vec![0, 255])));
    assert_eq!(lits[5], (CmpOp::Lt, FieldValue::Bool(true)));
    assert_eq!(lits[7], (CmpOp::Eq, FieldValue::Int(i64::MIN)));
    assert!(matches!(t[6], Expr::IsNull { negated: false, .. }) && matches!(t[7], Expr::IsNull { negated: true, .. }));
}

#[test]
fn a_query_prints_to_a_text_that_parses_to_the_same_query() {
    for text in [
        "FIND a",
        "FIND \"odd type\" AS o JOIN b VIA \"depends-on\" IN FROM o WHERE o.\"odd field\" = \"x\" ORDER BY b.k DESC LIMIT 0",
        "FIND a WHERE (x = 1 OR y = 2) AND (z = 3 AND w = 4) AND NOT (p = 1 OR q = 2) AND NOT NOT r = 5",
        "FIND a WHERE x = 1 OR (y = 2 OR z = 3) OR (u = 4 AND v = 5)",
        "FIND a WHERE s = \"tab\\t nul\\0 é\" AND f = 1e300 AND g = -0.0 AND h = 1e-7 AND b = x\"\" OFFSET 3",
        "FIND a JOIN b VIA r JOIN c VIA s OUT FROM a WHERE b.x IS NULL AND c.y IS NOT NULL LIMIT 4 OFFSET 2",
    ] {
        let q = parse(text);
        let printed = q.to_string();
        assert_eq!(parse(&printed), q, "{text:?} printed as {printed:?}");
        assert_eq!(parse(&printed).to_string(), printed);
    }
}

#[test]
fn a_keyword_is_a_name_only_as_a_string() {
    assert_eq!(parse("FIND \"where\" AS \"limit\"").root.alias, "limit");
    assert!(fails("FIND where").message.contains("keyword"));
    assert!(fails("FIND a WHERE order = 1").message.contains("keyword"));
    assert!(parse("FIND \"in\"").to_string().contains("\"in\""));
}

#[test]
fn a_parse_error_names_the_byte_and_the_reason() {
    let at = |text: &str, pos: usize, what: &str| {
        let e = fails(text);
        assert_eq!(e.pos, pos, "{text:?}: {e}");
        assert!(e.message.contains(what), "{text:?}: {e}");
    };
    at("", 0, "FIND");
    at("SELECT a", 0, "FIND");
    at("FIND", 4, "unit type");
    at("FIND a JOIN b", 13, "VIA");
    at("FIND a AS x JOIN b AS x VIA r", 22, "used twice");
    at("FIND a JOIN b VIA r FROM c", 25, "earlier alias");
    at("FIND a JOIN b AS c VIA r FROM c", 30, "earlier alias");
    at("FIND a WHERE", 12, "field");
    at("FIND a WHERE x", 14, "operator");
    at("FIND a WHERE x =", 16, "literal");
    at("FIND a WHERE z.x = 1", 13, "not an alias");
    at("FIND a WHERE (x = 1", 19, "\")\"");
    at("FIND a WHERE x = \"open", 17, "not closed");
    at("FIND a WHERE x = \"a\\q\"", 19, "unknown escape");
    at("FIND a WHERE x = x\"abc\"", 17, "hex");
    at("FIND a WHERE x = 1 2", 19, "end of the query");
    at("FIND a WHERE x = 12ab", 19, "letters");
    at("FIND a WHERE x = 99999999999999999999", 17, "64 bits");
    at("FIND a WHERE x = 1e999", 17, "finite");
    at("FIND a WHERE x = #", 17, "unexpected character");
    at("FIND a WHERE x ! 1", 15, "unexpected character");
    at("FIND a ORDER x", 13, "BY");
    at("FIND a LIMIT -1", 13, "not negative");
    at("FIND a LIMIT x", 13, "not negative");
    at("FIND a OFFSET 1 LIMIT 2", 16, "end of the query");
    at("FIND a WHERE x IS 1", 18, "NULL");
}

#[test]
fn the_limits_of_a_query_are_enforced() {
    let nested = |n: usize| format!("FIND a WHERE {}x = 1{}", "(".repeat(n), ")".repeat(n));
    assert!(Query::parse(&nested(MAX_DEPTH)).is_ok());
    assert!(fails(&nested(MAX_DEPTH + 1)).message.contains("nest"));
    let nots = |n: usize| format!("FIND a WHERE {}x = 1", "NOT ".repeat(n));
    assert!(Query::parse(&nots(MAX_DEPTH)).is_ok());
    assert!(fails(&nots(MAX_DEPTH + 1)).message.contains("nest"));
    let many = |n: usize| format!("FIND a WHERE {}", vec!["x = 1"; n].join(" AND "));
    assert!(Query::parse(&many(MAX_CONDITIONS)).is_ok());
    assert!(fails(&many(MAX_CONDITIONS + 1)).message.contains("conditions"));
    let long = format!("FIND a WHERE x = \"{}\"", "y".repeat(MAX_QUERY_LEN));
    assert!(fails(&long).message.contains("at most"));
}

#[test]
fn no_text_makes_the_parser_panic() {
    // Every prefix and every single-byte edit of valid queries, and bytes of every value.
    let seeds = [
        "FIND user AS u JOIN team AS t VIA member_of IN FROM u WHERE u.age >= 30 AND (t.name = \"é\" OR NOT x IS NULL) \
         ORDER BY u.age DESC LIMIT 10 OFFSET 2",
        "FIND a WHERE b = x\"00ff\" AND c = -1.5e-3",
    ];
    for seed in seeds {
        for end in 0..=seed.len() {
            if seed.is_char_boundary(end) {
                let _ = Query::parse(&seed[..end]);
            }
        }
        for at in 0..seed.len() {
            for byte in [0u8, b'"', b'\\', b'(', b'-', b'x', 0x7f, b'.'] {
                let mut raw = seed.as_bytes().to_vec();
                raw[at] = byte;
                if let Ok(s) = std::str::from_utf8(&raw) {
                    let _ = Query::parse(s);
                }
            }
        }
    }
    for c in (0..=0x10FFFFu32).step_by(97).filter_map(char::from_u32) {
        let _ = Query::parse(&format!("FIND a WHERE x = \"{c}\" AND {c} = 1 {c}"));
    }
}

#[test]
fn names_may_start_and_continue_with_an_underscore() {
    let q = parse("FIND _t_1 AS a_b WHERE _f = 1 AND a_b._g_ = 2");
    assert_eq!((q.root.unit_type.as_str(), q.root.alias.as_str()), ("_t_1", "a_b"));
    let Some(Expr::And(t)) = q.filter else { panic!("not a conjunction") };
    assert!(matches!(&t[0], Expr::Cmp { field, .. } if field.field == "_f"));
    assert!(matches!(&t[1], Expr::Cmp { field, .. } if field.field == "_g_"));
    assert_eq!(fails("FIND #t").message, "unexpected character '#'");
    assert_eq!(fails("FIND a WHERE x_ = 1 y").pos, 20);
}

#[test]
fn a_minus_or_a_dot_that_is_not_part_of_a_number_is_refused_where_it_stands() {
    let at = |text: &str, pos: usize, message: &str| {
        let e = fails(text);
        assert_eq!((e.pos, e.message.as_str()), (pos, message), "{text:?}");
    };
    at("FIND a WHERE x = -", 17, "unexpected character '-'");
    at("FIND a WHERE x = - 1", 17, "unexpected character '-'");
    at("FIND a WHERE x = -a", 17, "unexpected character '-'");
    // `1.` is the integer 1 and then a dot; `1.5` is one float; `1e` is the integer then letters.
    at("FIND a WHERE x = 1.", 18, "expected the end of the query, found \".\"");
    at("FIND a WHERE x = 1.a", 18, "expected the end of the query, found \".\"");
    at("FIND a WHERE x = 1e", 18, "a number is followed by letters");
    at("FIND a WHERE x = 1e+", 18, "a number is followed by letters");
    assert_eq!(parse("FIND a WHERE x = 1.5e+2").to_string(), "FIND a AS a WHERE a.x = 150.0");
}

#[test]
fn a_query_of_exactly_the_longest_length_parses_and_one_byte_more_does_not() {
    let head = "FIND a WHERE x = \"";
    let of_len = |n: usize| format!("{head}{}\"", "y".repeat(n - head.len() - 1));
    assert_eq!(of_len(MAX_QUERY_LEN).len(), MAX_QUERY_LEN);
    assert!(Query::parse(&of_len(MAX_QUERY_LEN)).is_ok());
    let e = fails(&of_len(MAX_QUERY_LEN + 1));
    assert_eq!((e.pos, e.message.as_str()), (MAX_QUERY_LEN, "a query is at most 16384 bytes"));
}

#[test]
fn a_missing_literal_says_what_a_literal_is() {
    let e = fails("FIND a WHERE x = )");
    assert_eq!((e.pos, e.message.as_str()), (17, "expected a literal: a number, a string, x\"..\", true or false"));
    assert_eq!(
        fails("FIND a WHERE x = null").message,
        "expected a literal: a number, a string, x\"..\", true or false"
    );
}
