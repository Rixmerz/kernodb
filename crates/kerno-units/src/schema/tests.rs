use std::cmp::Ordering;

use super::*;

/// The order of two values of one type, as a user means it.
fn cmp(a: &FieldValue, b: &FieldValue) -> Ordering {
    match (a, b) {
        (FieldValue::Bool(a), FieldValue::Bool(b)) => a.cmp(b),
        (FieldValue::Int(a), FieldValue::Int(b)) => a.cmp(b),
        (FieldValue::Float(a), FieldValue::Float(b)) => a.partial_cmp(b).expect("no NaN"),
        (FieldValue::Text(a), FieldValue::Text(b)) => a.as_bytes().cmp(b.as_bytes()),
        (FieldValue::Bytes(a), FieldValue::Bytes(b)) => a.cmp(b),
        _ => unreachable!("one type per list"),
    }
}

fn assert_order_preserved(values: &[FieldValue]) {
    for a in values {
        for b in values {
            let (ea, eb) = (a.index_bytes().expect("encodable"), b.index_bytes().expect("encodable"));
            assert_eq!(cmp(a, b), ea.cmp(&eb), "{a:?} against {b:?}");
            // Self-delimiting: no encoding is a proper prefix of another.
            assert!(ea == eb || !eb.starts_with(&ea), "{a:?} is a prefix of {b:?}");
        }
    }
}

#[test]
fn index_bytes_keep_the_order_of_every_type() {
    let ints = [i64::MIN, i64::MIN + 1, -256, -1, 0, 1, 255, 256, i64::MAX - 1, i64::MAX].map(FieldValue::Int);
    assert_order_preserved(&ints);
    let floats =
        [f64::NEG_INFINITY, -1e300, -1.5, -f64::MIN_POSITIVE, 0.0, f64::MIN_POSITIVE, 0.1, 2.0, 1e300, f64::INFINITY]
            .map(FieldValue::Float);
    assert_order_preserved(&floats);
    let texts = ["", "\0", "\0\0", "\0a", "a", "a\0", "a\0\0", "a\0b", "a\u{1}", "ab", "b", "\u{7f}", "é", "é\0"]
        .map(|s| FieldValue::Text(s.into()));
    assert_order_preserved(&texts);
    let bytes = [vec![], vec![0], vec![0, 0], vec![0, 255], vec![1], vec![255], vec![255, 0]].map(FieldValue::Bytes);
    assert_order_preserved(&bytes);
    assert_order_preserved(&[FieldValue::Bool(false), FieldValue::Bool(true)]);
}

#[test]
fn negative_zero_is_zero_and_nan_is_not_indexable() {
    assert_eq!(FieldValue::Float(-0.0).index_bytes(), FieldValue::Float(0.0).index_bytes());
    assert_eq!(FieldValue::Float(f64::NAN).index_bytes(), None);
}

#[test]
fn a_document_round_trips_and_is_canonical() {
    let d = Document::new()
        .with("z", FieldValue::Bytes(vec![0, 1]))
        .with("a", FieldValue::Text("é".into()))
        .with("m", FieldValue::Float(-0.5))
        .with("n", FieldValue::Int(i64::MIN))
        .with("b", FieldValue::Bool(false));
    let bytes = d.encode().expect("encode");
    assert_eq!(Document::decode(&bytes).expect("decode"), d);
    // The same fields given in another order are the same bytes.
    let e = Document::new()
        .with("b", FieldValue::Bool(false))
        .with("n", FieldValue::Int(i64::MIN))
        .with("m", FieldValue::Float(-0.5))
        .with("a", FieldValue::Text("é".into()))
        .with("z", FieldValue::Bytes(vec![0, 1]));
    assert_eq!(e.encode().expect("encode"), bytes);
    // Every proper prefix of a document is refused, as is every trailing byte.
    for n in 0..bytes.len() {
        assert!(Document::parse(&bytes[..n]).is_err(), "a prefix of {n} bytes");
    }
    assert!(Document::parse(&[bytes.clone(), vec![0]].concat()).is_err());
}

#[test]
fn a_schema_round_trips_and_normalises() {
    let schema = Schema::new(vec![
        FieldDef::new("b", FieldType::Text).unique(),
        FieldDef::new("a", FieldType::Int).required().indexed(),
        FieldDef::new("c", FieldType::Bytes),
    ])
    .expect("schema");
    assert_eq!(schema.fields().iter().map(|f| f.name.as_str()).collect::<Vec<_>>(), ["a", "b", "c"]);
    assert!(schema.field("b").is_some_and(|f| f.indexed), "unique implies indexed");
    assert_eq!(Schema::decode(&schema.encode()).expect("decode"), schema);
    assert!(Schema::new(vec![FieldDef::new("a", FieldType::Int), FieldDef::new("a", FieldType::Text)]).is_err());
    assert!(Schema::new(vec![FieldDef::new("", FieldType::Int)]).is_err());
    assert!(Schema::new(vec![FieldDef::new("a\0", FieldType::Int)]).is_err());
    assert!(Schema::new((0..=MAX_FIELDS).map(|i| FieldDef::new(format!("f{i}"), FieldType::Int)).collect()).is_err());
}

#[test]
fn a_damaged_schema_record_names_its_rule() {
    let rule = |v: &[u8]| Schema::decode(v).expect_err("must not decode").rule;
    assert_eq!(rule(&[]), UnitRule::Unit9);
    assert_eq!(rule(&[0, 0]), UnitRule::Unit6);
    assert_eq!(rule(&[2, 0]), UnitRule::Unit7);
    assert_eq!(rule(&[1, 1]), UnitRule::Unit8);
    assert_eq!(rule(&[1, 0, 0, 1]), UnitRule::Unit9);
    assert_eq!(rule(&[1, 0, 0, 0, 7]), UnitRule::Unit10);
    // One field `a`, type Int, flags: unique without indexed.
    assert_eq!(rule(&[1, 0, 0, 1, 1, b'a', 2, 0b010]), UnitRule::Unit12);
    // An unknown type.
    assert_eq!(rule(&[1, 0, 0, 1, 1, b'a', 9, 0]), UnitRule::Unit12);
    // Names out of order.
    assert_eq!(rule(&[1, 0, 0, 2, 1, b'b', 2, 0, 1, b'a', 2, 0]), UnitRule::Unit12);
}

#[test]
fn entry_keys_round_trip() {
    let unit = UnitId([7; 16]);
    let key = entry_key("t\0", "f", &[1, 2, 0, 0], unit);
    assert_eq!(decode_entry_key(&key).expect("decode"), ("t\0".into(), "f".into(), vec![1, 2, 0, 0], unit));
    assert_eq!(decode_schema_key(&schema_key("t\0")).expect("decode"), "t\0");
    for n in 0..key.len() - 17 {
        assert!(decode_entry_key(&key[..n]).is_err(), "a key cut to {n} bytes");
    }
    let (start, end) = prefix_bounds(&[0x79, 0xFF]);
    assert_eq!((start, end), (vec![0x79, 0xFF], vec![0x7A]));
}

#[test]
fn a_document_that_is_not_canonical_does_not_parse() {
    // One field `a`: the bytes of its value follow `tail`.
    let one = |ty: u8, value: &[u8]| [&[1, 0, 1, 1, b'a', ty][..], value].concat();
    assert!(Document::parse(&one(1, &[1])).is_ok());
    assert!(Document::parse(&one(1, &[2])).is_err(), "a Bool of 2");
    assert!(Document::parse(&one(9, &[0])).is_err(), "an unknown type");
    assert!(Document::parse(&one(3, &f64::NAN.to_be_bytes())).is_err(), "a NaN");
    assert!(Document::parse(&one(3, &(-0.0f64).to_be_bytes())).is_ok());
    assert!(Document::parse(&one(4, &[0, 0, 0, 1, 0xFF])).is_err(), "Text that is not UTF-8");
    assert!(Document::parse(&one(5, &[0, 0, 0, 1, 0xFF])).is_ok(), "Bytes need not be");
    assert!(Document::parse(&one(4, &[0, 0, 0, 5, b'a'])).is_err(), "a length past the end");
    // Names: repeated, out of order, not UTF-8, empty, with a NUL.
    assert!(Document::parse(&[1, 0, 2, 1, b'a', 1, 1, 1, b'a', 1, 1]).is_err());
    assert!(Document::parse(&[1, 0, 2, 1, b'b', 1, 1, 1, b'a', 1, 1]).is_err());
    assert!(Document::parse(&[1, 0, 1, 1, 0xFF, 1, 1]).is_err());
    assert!(Document::parse(&[1, 0, 1, 0, 1, 1]).is_err());
    assert!(Document::parse(&[1, 0, 1, 1, 0, 1, 1]).is_err());
    // The version byte and the field count.
    assert!(Document::parse(&[2, 0, 0]).is_err());
    assert!(Document::parse(&[1, 0, 0]).is_ok());
    assert!(Document::parse(&[1, 0, 65]).is_err());
    assert!(Document::parse(&[]).is_err());
}

#[test]
fn a_damaged_key_or_value_names_its_rule() {
    let unit = UnitId([7; 16]);
    let entry = |t: &[u8], f: &[u8], v: &[u8]| {
        [&[TAG_FIELD_INDEX, t.len() as u8][..], t, &[f.len() as u8], f, v, &unit.0].concat()
    };
    assert!(decode_entry_key(&entry(b"t", b"f", &[1])).is_ok());
    assert_eq!(decode_entry_key(&entry(b"", b"f", &[1])).unwrap_err().rule, UnitRule::Unit3);
    assert_eq!(decode_entry_key(&entry(b"t", b"", &[1])).unwrap_err().rule, UnitRule::Unit3);
    assert_eq!(decode_entry_key(&entry(b"t", b"f", &[])).unwrap_err().rule, UnitRule::Unit3, "no value");
    assert_eq!(decode_entry_key(&entry(&[b't'; 65], b"f", &[1])).unwrap_err().rule, UnitRule::Unit3);
    assert_eq!(decode_entry_key(&entry(b"t", &[b'f'; 65], &[1])).unwrap_err().rule, UnitRule::Unit3);
    assert_eq!(decode_entry_key(&entry(&[0xFF], b"f", &[1])).unwrap_err().rule, UnitRule::Unit12);
    assert_eq!(decode_entry_key(&entry(b"t", &[0xFF], &[1])).unwrap_err().rule, UnitRule::Unit12);
    // A schema key: tag, length, type.
    assert_eq!(decode_schema_key(&[TAG_SCHEMA, 1, b't']).unwrap(), "t");
    assert_eq!(decode_schema_key(&[TAG_SCHEMA, 0]).unwrap_err().rule, UnitRule::Unit3);
    assert_eq!(decode_schema_key(&[TAG_SCHEMA, 2, b't']).unwrap_err().rule, UnitRule::Unit3);
    assert_eq!(decode_schema_key(&[TAG_SCHEMA, 1, b't', b'u']).unwrap_err().rule, UnitRule::Unit3);
    assert_eq!(decode_schema_key(&[TAG_SCHEMA, 1, 0xFF]).unwrap_err().rule, UnitRule::Unit12);
    let long = [&[TAG_SCHEMA, 65][..], &[b't'; 65]].concat();
    assert_eq!(decode_schema_key(&long).unwrap_err().rule, UnitRule::Unit3);
    // An entry's value is the envelope and nothing else.
    assert!(check_entry_value(&[1, 0]).is_ok());
    let rule = |v: &[u8]| check_entry_value(v).unwrap_err().rule;
    assert_eq!(rule(&[]), UnitRule::Unit9);
    assert_eq!(rule(&[1]), UnitRule::Unit9);
    assert_eq!(rule(&[0, 0]), UnitRule::Unit6);
    assert_eq!(rule(&[2, 0]), UnitRule::Unit7);
    assert_eq!(rule(&[1, 1]), UnitRule::Unit8);
    assert_eq!(rule(&[1, 0, 0]), UnitRule::Unit10);
}

#[test]
fn a_unique_field_is_indexed_whichever_way_it_is_declared() {
    let literal = FieldDef { name: "u".into(), ty: FieldType::Int, required: false, unique: true, indexed: false };
    let schema = Schema::new(vec![literal]).expect("schema");
    assert!(schema.field("u").is_some_and(|f| f.unique && f.indexed));
}

#[test]
fn limits_hold_at_their_boundary() {
    let fields = |n: usize| (0..n).map(|i| FieldDef::new(format!("f{i:02}"), FieldType::Int)).collect::<Vec<_>>();
    assert!(Schema::new(fields(MAX_FIELDS)).is_ok());
    assert!(Schema::new(fields(MAX_FIELDS + 1)).is_err());
    // A stored schema is held to the same limit.
    let too_many = Schema { fields: fields(MAX_FIELDS + 1) };
    assert_eq!(Schema::decode(&too_many.encode()).expect_err("too many fields").rule, UnitRule::Unit12);
    assert!(Schema::decode(&Schema { fields: fields(MAX_FIELDS) }.encode()).is_ok());
    let doc = |n: usize| (0..n).fold(Document::new(), |d, i| d.with(format!("f{i:02}"), FieldValue::Int(i as i64)));
    assert!(Document::decode(&doc(MAX_FIELDS).encode().expect("encode")).is_ok());
    assert!(doc(MAX_FIELDS + 1).encode().is_err());
    let name = |n: usize| "n".repeat(n);
    assert!(Document::new().with(name(MAX_FIELD_NAME_LEN), FieldValue::Bool(true)).encode().is_ok());
    assert!(Document::new().with(name(MAX_FIELD_NAME_LEN + 1), FieldValue::Bool(true)).encode().is_err());
    // An indexed value is refused above 256 encoded bytes: text of 254 bytes encodes to 256.
    let schema = Schema::new(vec![FieldDef::new("t", FieldType::Text).indexed()]).expect("schema");
    let entries = |n: usize| schema.entries(&Document::new().with("t", FieldValue::Text("x".repeat(n))));
    assert_eq!(entries(MAX_INDEXED_VALUE_LEN - 2).expect("fits").len(), 1);
    assert!(entries(MAX_INDEXED_VALUE_LEN - 1).is_err());
}
