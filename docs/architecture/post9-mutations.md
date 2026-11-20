# Mutations after Stage 9

## P1: adjacency counters (K1)

**Method.** Each mutation was applied to a copy of the tree (without `target`, `.git` or `experiments`) with its own build directory (`CARGO_TARGET_DIR` inside the copy), only in release mode, one at a time; after each one the file was restored. Order: change the file, run `cargo test --release -p kerno-units --all-features --no-fail-fast` with the indicated test files, and note the `test … FAILED` lines as they were printed. The copy and its build directory were deleted at the end. The lines cited are those of the tree (the copy was identical).

**Test files run.** M1 to M16: `rel_counts`, `relations`, `assoc_indexed`. M17 and M19: `verify`, `unit_db`. M18: `assoc_reference`. All mutations came out red (`exit=101`); none survived.

**Test count of `kerno-units`** (`cargo test -p kerno-units --all-features --release -- --list`, lines `: test`): **822 before** (with the code of P1 but without new tests, measured at the start) and **853 after**: +31 from `rel_counts.rs` (30 run and 1 `#[ignore]` that regenerates the fixture; 14 tests per engine in two engines, plus the vector of the document, the fixture manifest and the regenerator). The other tests changed (`relations.rs`, `assoc_indexed.rs`, `assoc_reference.rs`, `verify.rs`, `unit_db.rs`) do not change their number; `completeness_is_not_a_query_time_check` was renamed.

**What test each mutation covers.** Each new or changed test fails under at least one:

| Test | Mutation |
|---|---|
| `one_entry_lost_{small,large}_store`, `one_of_several_entries_lost`, `a_count_that_disagrees_is_an_error` | M1, M4, M5, M11 |
| `one_adjacency_entry_missing_{small,large}_store`, `verifier_reports_index_damage` (relations.rs) | M1, M11 |
| `false_index_answers_…`, `a_relation_lost_with_both_entries_is_caught_by_its_counts` (assoc_indexed.rs) | M1 |
| `the_reference_path_never_reads_the_adjacency_index` (assoc_reference.rs) | M18 |
| `record_layout_is_pinned` | M2, M12 |
| `counts_follow_create_and_delete` | M2, M3, M14 |
| `an_aborted_change_set_leaves_no_count` | M15 (mutation of the test itself) |
| `a_refused_create_leaves_no_count` | M20 (later correction of K1, see below) |
| `a_count_lost_with_its_entry_is_the_known_limit` | M13 |
| `verifier_rules_of_the_count_records` | M5, M9, M10 |
| `legacy_relations_work_and_get_a_count_when_next_changed`, `deleting_a_legacy_relation_counts_what_is_left` | M3, M7 |
| `legacy_directory_opens_works_and_upgrades_when_written` | M2, M6 |
| `export_import_keeps_the_counts` | M8 |
| `vectors_of_the_document_match_the_codec_byte_for_byte` | M9 |
| `fixture_manifest_lists_the_committed_files` | M16 |
| `layer3_reader_refuses_a_database_with_counts` | M6, M14 |
| `open_and_mark`, `damage_detected` (unit_db.rs, verify.rs) | M17, M19 |
| `verify_writes_nothing`, `kerno_read_only_verify_untouched` (verify.rs) | M19 |

**Caveats.** (1) M15 mutates the test, not `src/`: the abort test rests on the engine's rollback, and no reasonable mutation of `src/` turns it red. (2) M16 alters one byte of the committed `MANIFEST`, not code. (3) Some mutations (M6, M12, M13) also turn red tests outside P1 (`create_and_read`, `differential_seeded`…), which is expected because they change the behaviour of every relation; they are listed as they came out. (4) M1 and M2 were run with a cold build. (5) The full check (`cargo test -p kerno-core -p kerno-storage -p kerno-testkit -p kerno-units --all-features --release --no-fail-fast`) finished with `exit=0` and 101 `test result` lines without failures, after correcting the tests that fixed the previous behaviour.

## Record

### M1: `crates/kerno-units/src/adj_count.rs:324`

- Change: `'        if got != *want {'` → `'        if false && got != *want {'`
- `exit=101`
- Red tests (20), lines as they were printed:

```text
test emulated_log::a_relation_lost_with_both_entries_is_caught_by_its_counts ... FAILED
test emulated_log::false_index_answers_are_detected_and_answered_through_the_fallback ... FAILED
test kerno::a_relation_lost_with_both_entries_is_caught_by_its_counts ... FAILED
test kerno::false_index_answers_are_detected_and_answered_through_the_fallback ... FAILED
test emulated_log::a_count_that_disagrees_is_an_error ... FAILED
test emulated_log::legacy_relations_work_and_get_a_count_when_next_changed ... FAILED
test emulated_log::one_entry_lost_small_store ... FAILED
test emulated_log::one_entry_lost_large_store ... FAILED
test emulated_log::one_of_several_entries_lost ... FAILED
test kerno::a_count_that_disagrees_is_an_error ... FAILED
test kerno::legacy_relations_work_and_get_a_count_when_next_changed ... FAILED
test kerno::one_entry_lost_small_store ... FAILED
test kerno::one_of_several_entries_lost ... FAILED
test kerno::one_entry_lost_large_store ... FAILED
test emulated_log::one_adjacency_entry_missing_small_store ... FAILED
test emulated_log::one_adjacency_entry_missing_large_store ... FAILED
test emulated_log::verifier_reports_index_damage ... FAILED
test kerno::one_adjacency_entry_missing_large_store ... FAILED
test kerno::one_adjacency_entry_missing_small_store ... FAILED
test kerno::verifier_reports_index_damage ... FAILED
```

### M2: `crates/kerno-units/src/db.rs:1487`

- Change: `'        self.adjust_count(to, Direction::In, relation_type, 1)?;\n'` → `''`
- `exit=101`
- Red tests (22), lines as they were printed:

```text
test emulated_log::counts_follow_create_and_delete ... FAILED
test emulated_log::export_import_keeps_the_counts ... FAILED
test emulated_log::legacy_directory_opens_works_and_upgrades_when_written ... FAILED
test emulated_log::legacy_relations_work_and_get_a_count_when_next_changed ... FAILED
test emulated_log::one_entry_lost_small_store ... FAILED
test emulated_log::one_of_several_entries_lost ... FAILED
test emulated_log::record_layout_is_pinned ... FAILED
test emulated_log::one_entry_lost_large_store ... FAILED
test kerno::counts_follow_create_and_delete ... FAILED
test kerno::export_import_keeps_the_counts ... FAILED
test kerno::legacy_relations_work_and_get_a_count_when_next_changed ... FAILED
test kerno::legacy_directory_opens_works_and_upgrades_when_written ... FAILED
test kerno::one_entry_lost_small_store ... FAILED
test kerno::record_layout_is_pinned ... FAILED
test kerno::one_of_several_entries_lost ... FAILED
test kerno::one_entry_lost_large_store ... FAILED
test emulated_log::one_adjacency_entry_missing_small_store ... FAILED
test emulated_log::one_adjacency_entry_missing_large_store ... FAILED
test emulated_log::differential_seeded ... FAILED
test kerno::differential_seeded ... FAILED
test kerno::one_adjacency_entry_missing_small_store ... FAILED
test kerno::one_adjacency_entry_missing_large_store ... FAILED
```

### M3: `crates/kerno-units/src/db.rs:1508`

- Change: `'        self.adjust_count(r.from, Direction::Out, &r.relation_type, -1)?;\n'` → `''`
- `exit=101`
- Red tests (12), lines as they were printed:

```text
test emulated_log::false_index_answers_are_detected_and_answered_through_the_fallback ... FAILED
test kerno::false_index_answers_are_detected_and_answered_through_the_fallback ... FAILED
test emulated_log::counts_follow_create_and_delete ... FAILED
test emulated_log::deleting_a_legacy_relation_counts_what_is_left ... FAILED
test kerno::counts_follow_create_and_delete ... FAILED
test kerno::deleting_a_legacy_relation_counts_what_is_left ... FAILED
test emulated_log::archive_and_delete_are_rejected_with_live_relations ... FAILED
test emulated_log::create_and_read ... FAILED
test emulated_log::differential_seeded ... FAILED
test kerno::archive_and_delete_are_rejected_with_live_relations ... FAILED
test kerno::differential_seeded ... FAILED
test kerno::create_and_read ... FAILED
```

### M4: `crates/kerno-units/src/adj_count.rs:303`

- Change: `'    let (start, end) = count_range(unit, dir);\n    scan_all('` → `'    let (start, end) = count_range(unit, dir);\n    if true {\n        return Ok(out);\n    }\n    scan_all('`
- `exit=101`
- Red tests (16), lines as they were printed:

```text
test emulated_log::a_relation_lost_with_both_entries_is_caught_by_its_counts ... FAILED
test emulated_log::false_index_answers_are_detected_and_answered_through_the_fallback ... FAILED
test kerno::a_relation_lost_with_both_entries_is_caught_by_its_counts ... FAILED
test kerno::false_index_answers_are_detected_and_answered_through_the_fallback ... FAILED
test emulated_log::a_count_that_disagrees_is_an_error ... FAILED
test emulated_log::legacy_relations_work_and_get_a_count_when_next_changed ... FAILED
test emulated_log::one_entry_lost_large_store ... FAILED
test emulated_log::one_entry_lost_small_store ... FAILED
test emulated_log::one_of_several_entries_lost ... FAILED
test kerno::a_count_that_disagrees_is_an_error ... FAILED
test kerno::legacy_relations_work_and_get_a_count_when_next_changed ... FAILED
test kerno::one_of_several_entries_lost ... FAILED
test kerno::one_entry_lost_small_store ... FAILED
test kerno::one_entry_lost_large_store ... FAILED
test emulated_log::verifier_reports_index_damage ... FAILED
test kerno::verifier_reports_index_damage ... FAILED
```

### M5: `crates/kerno-units/src/verify.rs:944`

- Change: `'            if got != *want {\n                self.cross(\n                    CrossRule::Unit45'` → `'            if false && got != *want {\n                self.cross(\n                    CrossRule::Unit45'`
- `exit=101`
- Red tests (8), lines as they were printed:

```text
test emulated_log::a_count_that_disagrees_is_an_error ... FAILED
test emulated_log::one_entry_lost_small_store ... FAILED
test emulated_log::one_of_several_entries_lost ... FAILED
test emulated_log::one_entry_lost_large_store ... FAILED
test kerno::a_count_that_disagrees_is_an_error ... FAILED
test kerno::one_entry_lost_small_store ... FAILED
test kerno::one_of_several_entries_lost ... FAILED
test kerno::one_entry_lost_large_store ... FAILED
```

### M6: `crates/kerno-units/src/db.rs:1572`

- Change: `'        let wants = if self.count_ops > 0 {'` → `'        let wants = if false && self.count_ops > 0 {'`
- `exit=101`
- Red tests (52), lines as they were printed:

```text
test emulated_log::a_relation_lost_with_both_entries_is_caught_by_its_counts ... FAILED
test emulated_log::false_index_answers_are_detected_and_answered_through_the_fallback ... FAILED
test emulated_log::differential_seeded ... FAILED
test kerno::a_relation_lost_with_both_entries_is_caught_by_its_counts ... FAILED
test kerno::differential_seeded ... FAILED
test kerno::false_index_answers_are_detected_and_answered_through_the_fallback ... FAILED
test emulated_log::frozen_corpus_indexed_equals_reference ... FAILED
test kerno::frozen_corpus_indexed_equals_reference ... FAILED
test emulated_log::a_count_that_disagrees_is_an_error ... FAILED
test emulated_log::counts_follow_create_and_delete ... FAILED
test emulated_log::deleting_a_legacy_relation_counts_what_is_left ... FAILED
test emulated_log::export_import_keeps_the_counts ... FAILED
test emulated_log::legacy_relations_work_and_get_a_count_when_next_changed ... FAILED
test emulated_log::layer3_reader_refuses_a_database_with_counts ... FAILED
test emulated_log::legacy_directory_opens_works_and_upgrades_when_written ... FAILED
test emulated_log::one_entry_lost_small_store ... FAILED
test emulated_log::record_layout_is_pinned ... FAILED
test emulated_log::one_of_several_entries_lost ... FAILED
test emulated_log::verifier_rules_of_the_count_records ... FAILED
test kerno::a_count_that_disagrees_is_an_error ... FAILED
test emulated_log::one_entry_lost_large_store ... FAILED
test kerno::deleting_a_legacy_relation_counts_what_is_left ... FAILED
test kerno::counts_follow_create_and_delete ... FAILED
test kerno::layer3_reader_refuses_a_database_with_counts ... FAILED
test kerno::export_import_keeps_the_counts ... FAILED
test kerno::legacy_relations_work_and_get_a_count_when_next_changed ... FAILED
test kerno::legacy_directory_opens_works_and_upgrades_when_written ... FAILED
test kerno::one_entry_lost_small_store ... FAILED
test kerno::one_entry_lost_large_store ... FAILED
test kerno::one_of_several_entries_lost ... FAILED
test kerno::record_layout_is_pinned ... FAILED
test kerno::verifier_rules_of_the_count_records ... FAILED
test emulated_log::create_and_read ... FAILED
test emulated_log::one_adjacency_entry_missing_large_store ... FAILED
test emulated_log::differential_across_scan_pages ... FAILED
test emulated_log::differential_seeded ... FAILED
test emulated_log::one_adjacency_entry_missing_small_store ... FAILED
test emulated_log::relation_only_changeset_commits ... FAILED
test emulated_log::provenance_is_stored_and_checked ... FAILED
test emulated_log::stored_provenance_corruption_is_detected ... FAILED
test emulated_log::relations_count_against_the_changeset_limit ... FAILED
test emulated_log::verifier_reports_index_damage ... FAILED
test kerno::create_and_read ... FAILED
test kerno::one_adjacency_entry_missing_small_store ... FAILED
test kerno::one_adjacency_entry_missing_large_store ... FAILED
test kerno::relation_only_changeset_commits ... FAILED
test kerno::provenance_is_stored_and_checked ... FAILED
test kerno::relations_count_against_the_changeset_limit ... FAILED
test kerno::verifier_reports_index_damage ... FAILED
test kerno::stored_provenance_corruption_is_detected ... FAILED
test kerno::differential_across_scan_pages ... FAILED
test kerno::differential_seeded ... FAILED
```

### M7: `crates/kerno-units/src/adj_count.rs:263`

- Change: `'    let Some((start, end)) = Key::adjacency_range(adjacency_tag(dir), unit, Some(relation_type)) else {\n        return Ok(0);\n    };'` → `'    let Some((start, end)) = Key::adjacency_range(adjacency_tag(dir), unit, Some(relation_type)) else {\n        return Ok(0);\n    };\n    if true {\n        return Ok(0);\n    }'`
- `exit=101`
- Red tests (4), lines as they were printed:

```text
test emulated_log::deleting_a_legacy_relation_counts_what_is_left ... FAILED
test emulated_log::legacy_relations_work_and_get_a_count_when_next_changed ... FAILED
test kerno::deleting_a_legacy_relation_counts_what_is_left ... FAILED
test kerno::legacy_relations_work_and_get_a_count_when_next_changed ... FAILED
```

### M8: `crates/kerno-units/src/import.rs:213`

- Change: `'    cs.adjust_count(r.from, crate::types::Direction::Out, ty, 1)?;\n    cs.adjust_count(r.to, crate::types::Direction::In, ty, 1)?;\n'` → `''`
- `exit=101`
- Red tests (2), lines as they were printed:

```text
test emulated_log::export_import_keeps_the_counts ... FAILED
test kerno::export_import_keeps_the_counts ... FAILED
```

### M9: `crates/kerno-units/src/adj_count.rs:153`

- Change: `'    if count == 0 {'` → `'    if false && count == 0 {'`
- `exit=101`
- Red tests (3), lines as they were printed:

```text
test emulated_log::verifier_rules_of_the_count_records ... FAILED
test kerno::verifier_rules_of_the_count_records ... FAILED
test vectors_of_the_document_match_the_codec_byte_for_byte ... FAILED
```

### M10: `crates/kerno-units/src/verify.rs:922`

- Change: `'            && mark.layer_version < crate::codec::LAYER_VERSION_REL_COUNTS'` → `'            && mark.layer_version < 0'`
- `exit=101`
- Red tests (2), lines as they were printed:

```text
test emulated_log::verifier_rules_of_the_count_records ... FAILED
test kerno::verifier_rules_of_the_count_records ... FAILED
```

### M11: `crates/kerno-units/src/planner.rs:830`

- Change: `'                Err(e) if is_corruption(&e) => {\n                    fallback = Some(Fallback { abandoned: chosen, cause: FallbackCause::Corrupt });\n                }\n                Err(e) => return Err(e),\n            }\n        }\n        Ok(RelationsOutcome {'` → `'                Err(e) if false && is_corruption(&e) => {\n                    fallback = Some(Fallback { abandoned: chosen, cause: FallbackCause::Corrupt });\n                }\n                Err(e) => return Err(e),\n            }\n        }\n        Ok(RelationsOutcome {'`
- `exit=101`
- Red tests (10), lines as they were printed:

```text
test emulated_log::a_count_that_disagrees_is_an_error ... FAILED
test emulated_log::one_entry_lost_small_store ... FAILED
test emulated_log::one_entry_lost_large_store ... FAILED
test kerno::a_count_that_disagrees_is_an_error ... FAILED
test kerno::one_entry_lost_small_store ... FAILED
test kerno::one_entry_lost_large_store ... FAILED
test emulated_log::one_adjacency_entry_missing_small_store ... FAILED
test emulated_log::one_adjacency_entry_missing_large_store ... FAILED
test kerno::one_adjacency_entry_missing_small_store ... FAILED
test kerno::one_adjacency_entry_missing_large_store ... FAILED
```

### M12: `crates/kerno-units/src/adj_count.rs:79`

- Change: `'        Direction::In => 1,\n    }\n}\n\npub(crate) fn dir_of'` → `'        Direction::In => 0,\n    }\n}\n\npub(crate) fn dir_of'`
- `exit=101`
- Red tests (51), lines as they were printed:

```text
test emulated_log::a_relation_lost_with_both_entries_is_caught_by_its_counts ... FAILED
test emulated_log::differential_seeded ... FAILED
test emulated_log::false_index_answers_are_detected_and_answered_through_the_fallback ... FAILED
test emulated_log::differential_across_scan_pages ... FAILED
test emulated_log::observations_record_the_path_the_rows_and_the_class_of_an_error ... FAILED
test kerno::a_relation_lost_with_both_entries_is_caught_by_its_counts ... FAILED
test kerno::differential_seeded ... FAILED
test kerno::false_index_answers_are_detected_and_answered_through_the_fallback ... FAILED
test kerno::observations_record_the_path_the_rows_and_the_class_of_an_error ... FAILED
test kerno::differential_across_scan_pages ... FAILED
test kerno::frozen_corpus_indexed_equals_reference ... FAILED
test emulated_log::frozen_corpus_indexed_equals_reference ... FAILED
test emulated_log::counts_follow_create_and_delete ... FAILED
test emulated_log::a_count_that_disagrees_is_an_error ... FAILED
test emulated_log::export_import_keeps_the_counts ... FAILED
test emulated_log::legacy_directory_opens_works_and_upgrades_when_written ... FAILED
test emulated_log::legacy_relations_work_and_get_a_count_when_next_changed ... FAILED
test emulated_log::one_entry_lost_small_store ... FAILED
test emulated_log::record_layout_is_pinned ... FAILED
test emulated_log::verifier_rules_of_the_count_records ... FAILED
test kerno::a_count_that_disagrees_is_an_error ... FAILED
test kerno::counts_follow_create_and_delete ... FAILED
test kerno::export_import_keeps_the_counts ... FAILED
test kerno::legacy_directory_opens_works_and_upgrades_when_written ... FAILED
test kerno::legacy_relations_work_and_get_a_count_when_next_changed ... FAILED
test kerno::one_entry_lost_small_store ... FAILED
test kerno::record_layout_is_pinned ... FAILED
test emulated_log::one_entry_lost_large_store ... FAILED
test kerno::verifier_rules_of_the_count_records ... FAILED
test vectors_of_the_document_match_the_codec_byte_for_byte ... FAILED
test kerno::one_entry_lost_large_store ... FAILED
test emulated_log::create_and_read ... FAILED
test emulated_log::differential_seeded ... FAILED
test emulated_log::one_adjacency_entry_missing_large_store ... FAILED
test emulated_log::one_adjacency_entry_missing_small_store ... FAILED
test emulated_log::differential_across_scan_pages ... FAILED
test emulated_log::provenance_is_stored_and_checked ... FAILED
test emulated_log::relations_count_against_the_changeset_limit ... FAILED
test emulated_log::relation_only_changeset_commits ... FAILED
test emulated_log::stored_provenance_corruption_is_detected ... FAILED
test emulated_log::verifier_reports_index_damage ... FAILED
test kerno::create_and_read ... FAILED
test kerno::differential_seeded ... FAILED
test kerno::one_adjacency_entry_missing_small_store ... FAILED
test kerno::relation_only_changeset_commits ... FAILED
test kerno::one_adjacency_entry_missing_large_store ... FAILED
test kerno::provenance_is_stored_and_checked ... FAILED
test kerno::differential_across_scan_pages ... FAILED
test kerno::relations_count_against_the_changeset_limit ... FAILED
test kerno::stored_provenance_corruption_is_detected ... FAILED
test kerno::verifier_reports_index_damage ... FAILED
```

### M13: `crates/kerno-units/src/adj_count.rs:322`

- Change: `'    for (t, want) in expected {\n'` → `'    if expected.is_empty() {\n        return Err(crate::db::corruption("mutation".into()));\n    }\n    for (t, want) in expected {\n'`
- `exit=101`
- Red tests (26), lines as they were printed:

```text
test emulated_log::a_relation_lost_with_both_entries_is_caught_by_its_counts ... FAILED
test emulated_log::differential_seeded ... FAILED
test emulated_log::false_index_answers_are_detected_and_answered_through_the_fallback ... FAILED
test emulated_log::differential_across_scan_pages ... FAILED
test emulated_log::observations_record_the_path_the_rows_and_the_class_of_an_error ... FAILED
test kerno::a_relation_lost_with_both_entries_is_caught_by_its_counts ... FAILED
test kerno::differential_seeded ... FAILED
test kerno::differential_across_scan_pages ... FAILED
test kerno::observations_record_the_path_the_rows_and_the_class_of_an_error ... FAILED
test kerno::false_index_answers_are_detected_and_answered_through_the_fallback ... FAILED
test kerno::frozen_corpus_indexed_equals_reference ... FAILED
test emulated_log::frozen_corpus_indexed_equals_reference ... FAILED
test emulated_log::a_count_lost_with_its_entry_is_the_known_limit ... FAILED
test emulated_log::legacy_relations_work_and_get_a_count_when_next_changed ... FAILED
test emulated_log::legacy_directory_opens_works_and_upgrades_when_written ... FAILED
test kerno::a_count_lost_with_its_entry_is_the_known_limit ... FAILED
test kerno::legacy_directory_opens_works_and_upgrades_when_written ... FAILED
test kerno::legacy_relations_work_and_get_a_count_when_next_changed ... FAILED
test emulated_log::abort_leaves_no_relation ... FAILED
test emulated_log::create_and_read ... FAILED
test emulated_log::differential_seeded ... FAILED
test emulated_log::differential_across_scan_pages ... FAILED
test kerno::abort_leaves_no_relation ... FAILED
test kerno::create_and_read ... FAILED
test kerno::differential_seeded ... FAILED
test kerno::differential_across_scan_pages ... FAILED
```

### M14: `crates/kerno-units/src/codec.rs:58`

- Change: `'pub const LAYER_VERSION_REL_COUNTS: u32 = 4;'` → `'pub const LAYER_VERSION_REL_COUNTS: u32 = 3;'`
- `exit=101`
- Red tests (17), lines as they were printed:

```text
test emulated_log::deleting_a_legacy_relation_counts_what_is_left ... FAILED
test emulated_log::counts_follow_create_and_delete ... FAILED
test emulated_log::layer3_reader_refuses_a_database_with_counts ... FAILED
test emulated_log::export_import_keeps_the_counts ... FAILED
test emulated_log::legacy_directory_opens_works_and_upgrades_when_written ... FAILED
test emulated_log::legacy_relations_work_and_get_a_count_when_next_changed ... FAILED
test emulated_log::record_layout_is_pinned ... FAILED
test emulated_log::verifier_rules_of_the_count_records ... FAILED
test kerno::deleting_a_legacy_relation_counts_what_is_left ... FAILED
test kerno::counts_follow_create_and_delete ... FAILED
test kerno::layer3_reader_refuses_a_database_with_counts ... FAILED
test kerno::export_import_keeps_the_counts ... FAILED
test kerno::legacy_directory_opens_works_and_upgrades_when_written ... FAILED
test kerno::legacy_relations_work_and_get_a_count_when_next_changed ... FAILED
test kerno::record_layout_is_pinned ... FAILED
test kerno::verifier_rules_of_the_count_records ... FAILED
test vectors_of_the_document_match_the_codec_byte_for_byte ... FAILED
```

### M15: `crates/kerno-units/tests/rel_counts.rs:172`

- Change: `'    drop(cs);\n    assert!(count_records(&db).is_empty());'` → `'    drop(cs);\n    assert!(!count_records(&db).is_empty());'`
- `exit=101`
- Red tests (2), lines as they were printed:

```text
test emulated_log::an_aborted_change_set_leaves_no_count ... FAILED
test kerno::an_aborted_change_set_leaves_no_count ... FAILED
```

### M16: `crates/kerno-units/tests/vectors/rel-counts-v4/MANIFEST:1`

- Change: `None` → `None`
- `exit=101`
- Red tests (1), lines as they were printed:

```text
test fixture_manifest_lists_the_committed_files ... FAILED
```

### M17 (extra): `crates/kerno-units/src/codec.rs:60`

- Change: `'pub const MAX_LAYER_VERSION: u32 = LAYER_VERSION_REL_COUNTS;'` → `'pub const MAX_LAYER_VERSION: u32 = LAYER_VERSION_PUBLIC_IDS;'`
- `exit=101`
- Red tests (4), lines as they were printed:

```text
test emulated_log::open_and_mark ... FAILED
test kerno::open_and_mark ... FAILED
test emulated_log::damage_detected ... FAILED
test kerno::damage_detected ... FAILED
```

### M18: `crates/kerno-units/src/adj_count.rs:324`

- Change: `'        if got != *want {'` → `'        if false && got != *want {'`
- `exit=101`
- Red tests (2), lines as they were printed:

```text
test emulated_log::the_reference_path_never_reads_the_adjacency_index ... FAILED
test kerno::the_reference_path_never_reads_the_adjacency_index ... FAILED
```

### M19: `crates/kerno-units/src/codec.rs:60`

- Change: `'pub const MAX_LAYER_VERSION: u32 = LAYER_VERSION_REL_COUNTS;'` → `'pub const MAX_LAYER_VERSION: u32 = 5;'`
- `exit=101`
- Red tests (7), lines as they were printed:

```text
test emulated_log::open_and_mark ... FAILED
test kerno::open_and_mark ... FAILED
test emulated_log::verify_writes_nothing ... FAILED
test emulated_log::damage_detected ... FAILED
test kerno::verify_writes_nothing ... FAILED
test kerno::damage_detected ... FAILED
test kerno_read_only_verify_untouched ... FAILED
```

## Later correction of K1: the rejection of `encode` leaves no counters

An independent review found that `create_relation` adjusted the two counters before `record.encode()` validated the provenance: if `encode` rejected the relation, the adjusted counter stayed in the open change set and a later `commit` wrote it, and `relations_of` gave `Corruption` and `verify_units` UNIT-45 on a store without damage. The correction builds and encodes all the writes before touching the counters (`db.rs:1473-1487`); `delete_relation` does the same with the keys (`db.rs:1503-1509`). The test `a_refused_create_leaves_no_count` (rel_counts.rs, two engines: `Imported` with an empty source refused, then a valid relation and `commit`) was seen red before the correction in both engines (`Corruption` in `relations_of`).

Method as above (copy of the tree, its own `CARGO_TARGET_DIR`, release, one mutation), with `rel_counts`.

### M20: `crates/kerno-units/src/db.rs:1486`

- Change: the two lines `adjust_count(from, …, 1)` and `adjust_count(to, …, 1)` are moved before the block that encodes the writes (the order before the correction).
- `exit=101`
- Red tests (2), lines as they were printed:

```text
test emulated_log::a_refused_create_leaves_no_count ... FAILED
test kerno::a_refused_create_leaves_no_count ... FAILED
```

The other paths that adjust counters were audited: `delete_relation` (after the correction its only errors before writing are reads and keys; the errors of `adjust_count` only occur with a damaged counter record or an engine failure; since the F1 correction the two counter reads of `create_relation`, `delete_relation` and import are done before writing either of them, so a read error writes nothing and the change set stays sound, and only a `write` failure leaves it broken), import (`import.rs:213`: it had the same shape —it encoded the record after adjusting the counters—, but the change set is private to `replay` and is abandoned without `commit` on any error, so the damage was not reachable and admits no test; it was reordered anyway so that it builds and encodes before touching the counters), deletion and forgetting of units (they do not adjust counters: `ensure_no_relations` requires that there are no relations), and batches (`DerivedChangeSet::create_relation` delegates to `create_relation`).

## P2: missing tests and one text

Package P2 (points 3, 4, 5, 6, 10 and 11 of [stage9-closure.md](stage9-closure.md) §5; status in [post9-pending.md](post9-pending.md)).

**Method.** As for P1: a copy of the tree (without `target`, `.git` or `experiments`) with its own `CARGO_TARGET_DIR`, release only (with `CARGO_PROFILE_RELEASE_DEBUG=0` to save disk), one mutation at a time, the file restored after each one and the copy deleted at the end. Each mutation replaces a text that appears only once in its file; the line is that of that text in the tree. Q2, Q3, Q4, Q7, Q13c and Q15 were run with `--no-fail-fast` because their command covers several test binaries and, without that option, `cargo` stops at the first one that fails. All the mutations of the new or changed tests came out red (`exit=101`); the only one green is Q13b, which is not a mutation of a P2 test but a discarded candidate (see Caveats).

**Test count** (`cargo test -p kerno-core -p kerno-storage -p kerno-testkit -p kerno-units --all-features --release -- --list`, lines `: test`): **1 798** in the base tree and **1 804** with P2 (+6): `compact_asks_for_the_revision_ranges`, `compact_refuses_a_poisoned_handle_emulated_log`, `compact_refuses_a_poisoned_handle_kerno_db` (`reclaim.rs`), `db::accessor_tests::compact_covers_exactly_the_revision_tags_kerno_db` (`db.rs`), `emulated_log_registry_is_locked_while_the_engine_commits` and `kerno_db_registry_is_locked_while_the_engine_commits` (`derived_lock_order.rs`, new). The changed tests (`consolidation.rs`, `derived_stale.rs`, `redaction_units.rs`, and the two space tests of the harness) change their body, not their number.

**What test each mutation covers.**

| Test | Mutation |
|---|---|
| `compact_refuses_a_poisoned_handle_{emulated_log,kerno_db}` (`reclaim.rs`) | Q1 |
| `compact_asks_for_the_revision_ranges` (`reclaim.rs`), `compact_covers_exactly_the_revision_tags_kerno_db` (`db.rs`) | Q2, Q3, Q4 |
| `{emulated_log,kerno_db}_registry_is_locked_while_the_engine_commits` (`derived_lock_order.rs`) | Q5, Q6 |
| `errors_say_what_happened_without_content` (`consolidation.rs`), `every_error_says_what_it_is_without_naming_content` (`derived_stale.rs`) | Q7 |
| `units_leak_no_content_in_errors_reports_or_observations` (`redaction_units.rs`), leak assertion | Q8, Q9 |
| the same, assertion `payload_findings >= 2` | Q10 |
| the same, assertion `shape_texts >= 1` | Q11 |
| `quality_over_the_frozen_set` (`derived_measure.rs`), P4 (`derived_measure.rs:749`) | Q12 |
| the same, identical report between engines (`derived_measure.rs:747`) | Q13 |
| `m0_profile::shared_range::space_is_reclaimed_after_range_deletes`, key assertion | Q14 |
| `space_is_reclaimed_after_range_deletes` and `space_is_reclaimed`, all profiles, size assertion after the wait of `settled_size` | Q15 |
| `space_check_fails_for_an_engine_that_reclaims_nothing` and `space_check_fails_for_an_engine_that_keeps_range_deleted_points` (the harness's own tests), the single measurement of `settled_size` | Q16 |

**Caveats.**

1. **Q13 mutates the test, not `src/`.** All the code of `kerno-units` that walks this measurement is generic over the engine and has no branch that depends on it (apart from `compact()`, which the measurement does not call, it neither reads the engine's name nor its statistics); a mutation in `kerno-units` applies equally to both engines and leaves the two reports equal, as Q12 shows (P4 fails at line 749, not the equality at 747). A difference can only come from an engine that departs from the contract. A candidate of that kind was tried in `kerno-storage`, Q13b (in a transaction, with equal keys the base wins and not what the transaction itself wrote): the measurement stays green because none of its transactions reads by range a key that it wrote itself, and the engine's transaction conformance tests do see it (Q13c, 6 failures). That is why the assertion at 747 is shown with Q13, a figure of the report that depends on the engine.
2. Q5 moves the taking of the lock after the commit; no mutation shows that the lock stays taken during the marking (see point 4 of the pending items). Q6 mutates the hook `hook_registry_locked` (only with `test-support`), not production code: it shows that the test reads the hook's answer.
3. Q14 turns red the key assertion (`range.rs:609`, 0.12 s) and does not reach the size one. The size assertion and the wait of `settled_size` are exercised by Q15, which takes about 30 s per test because the wait runs out with an engine that does not reclaim; Q15 is what shows that the wait does not hide an engine that does not reclaim.
4. Q2 to Q4 are three ways of widening the range (starting one byte earlier, ending one byte later, no end). The `db.rs` test only runs in `kerno_db` (the tombstone counters are its own); the one in `reclaim.rs` sees the exact range in the reference engine, behind the wrapper that implements `Reclaim`.

## Record P2

### Q1: `crates/kerno-units/src/retention.rs:671`

- Change: `        self.check_poisoned()?;\n        let before = stored_bytes` → `        let before = stored_bytes`
- Command (in the copy): `cargo test --release -p kerno-units --all-features --test reclaim`
- `exit=101`
- Lines as they were printed:

```text
test compact_refuses_a_poisoned_handle_emulated_log ... FAILED
test compact_refuses_a_poisoned_handle_kerno_db ... FAILED
thread 'compact_refuses_a_poisoned_handle_emulated_log' (2372) panicked at crates/kerno-units/tests/reclaim.rs:443:5:
thread 'compact_refuses_a_poisoned_handle_kerno_db' (2373) panicked at crates/kerno-units/tests/reclaim.rs:443:5:
test result: FAILED. 11 passed; 2 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.07s
```

### Q2: `crates/kerno-units/src/retention.rs:675`

- Change: `Bound::Included(&[TAG_REV_META]),` → `Bound::Included(&[TAG_REV_META - 1]),`
- Command (in the copy): `cargo test --release --no-fail-fast -p kerno-units --all-features --lib --test reclaim`
- `exit=101`
- Lines as they were printed:

```text
test db::accessor_tests::compact_covers_exactly_the_revision_tags_kerno_db ... FAILED
thread 'db::accessor_tests::compact_covers_exactly_the_revision_tags_kerno_db' (12880) panicked at crates/kerno-units/src/db.rs:1815:13:
test result: FAILED. 65 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.13s
test compact_asks_for_the_revision_ranges ... FAILED
thread 'compact_asks_for_the_revision_ranges' (12998) panicked at crates/kerno-units/tests/reclaim.rs:427:5:
test result: FAILED. 12 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.07s
```

### Q3: `crates/kerno-units/src/retention.rs:676`

- Change: `Bound::Excluded(&[TAG_REV_PAYLOAD + 1]),` → `Bound::Excluded(&[TAG_REV_PAYLOAD + 2]),`
- Command (in the copy): `cargo test --release --no-fail-fast -p kerno-units --all-features --lib --test reclaim`
- `exit=101`
- Lines as they were printed:

```text
test db::accessor_tests::compact_covers_exactly_the_revision_tags_kerno_db ... FAILED
thread 'db::accessor_tests::compact_covers_exactly_the_revision_tags_kerno_db' (13311) panicked at crates/kerno-units/src/db.rs:1815:13:
test result: FAILED. 65 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.13s
test compact_asks_for_the_revision_ranges ... FAILED
thread 'compact_asks_for_the_revision_ranges' (13385) panicked at crates/kerno-units/tests/reclaim.rs:427:5:
test result: FAILED. 12 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.07s
```

### Q4: `crates/kerno-units/src/retention.rs:676`

- Change: `Bound::Excluded(&[TAG_REV_PAYLOAD + 1]),` → `Bound::Unbounded,`
- Command (in the copy): `cargo test --release --no-fail-fast -p kerno-units --all-features --lib --test reclaim`
- `exit=101`
- Lines as they were printed:

```text
test db::accessor_tests::compact_covers_exactly_the_revision_tags_kerno_db ... FAILED
thread 'db::accessor_tests::compact_covers_exactly_the_revision_tags_kerno_db' (13743) panicked at crates/kerno-units/src/db.rs:1815:13:
test result: FAILED. 65 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.12s
test compact_asks_for_the_revision_ranges ... FAILED
thread 'compact_asks_for_the_revision_ranges' (13817) panicked at crates/kerno-units/tests/reclaim.rs:427:5:
test result: FAILED. 12 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.07s
```

### Q5: `crates/kerno-units/src/derived_stale.rs:568`

- Change: `        let mut inner = self.owner.lock();\n        let info = self.cs.commit()?;` → `        let info = self.cs.commit()?;\n        let mut inner = self.owner.lock();`
- Command (in the copy): `cargo test --release -p kerno-units --all-features --test derived_lock_order`
- `exit=101`
- Lines as they were printed:

```text
test emulated_log_registry_is_locked_while_the_engine_commits ... FAILED
test kerno_db_registry_is_locked_while_the_engine_commits ... FAILED
thread 'emulated_log_registry_is_locked_while_the_engine_commits' (3825) panicked at crates/kerno-units/tests/derived_lock_order.rs:147:5:
thread 'kerno_db_registry_is_locked_while_the_engine_commits' (3826) panicked at crates/kerno-units/tests/derived_lock_order.rs:147:5:
test result: FAILED. 0 passed; 2 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.09s
```

### Q6: `crates/kerno-units/src/derived_stale.rs:387`

- Change: `        matches!(self.inner.try_lock(), Err(std::sync::TryLockError::WouldBlock))` → `        !matches!(self.inner.try_lock(), Err(std::sync::TryLockError::WouldBlock))`
- Command (in the copy): `cargo test --release -p kerno-units --all-features --test derived_lock_order`
- `exit=101`
- Lines as they were printed:

```text
test emulated_log_registry_is_locked_while_the_engine_commits ... FAILED
test kerno_db_registry_is_locked_while_the_engine_commits ... FAILED
thread 'emulated_log_registry_is_locked_while_the_engine_commits' (4223) panicked at crates/kerno-units/tests/derived_lock_order.rs:136:5:
thread 'kerno_db_registry_is_locked_while_the_engine_commits' (4224) panicked at crates/kerno-units/tests/derived_lock_order.rs:136:5:
test result: FAILED. 0 passed; 2 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.10s
```

### Q7: `crates/kerno-units/src/derived_stale.rs:124`

- Change: `is valid any more", IdHex(id))` → `is valid any more", IdHex(&DerivedId([0; 32])))`
- Command (in the copy): `cargo test --release --no-fail-fast -p kerno-units --all-features --test consolidation --test derived_stale`
- `exit=101`
- Lines as they were printed:

```text
test emulated_log::errors_say_what_happened_without_content ... FAILED
test kerno_db::errors_say_what_happened_without_content ... FAILED
thread 'emulated_log::errors_say_what_happened_without_content' (14755) panicked at crates/kerno-units/tests/consolidation.rs:496:5:
thread 'kerno_db::errors_say_what_happened_without_content' (14770) panicked at crates/kerno-units/tests/consolidation.rs:496:5:
test result: FAILED. 28 passed; 2 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.07s
test every_error_says_what_it_is_without_naming_content ... FAILED
thread 'every_error_says_what_it_is_without_naming_content' (14799) panicked at crates/kerno-units/tests/derived_stale.rs:611:9:
test result: FAILED. 45 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.08s
```

### Q8: `crates/kerno-units/src/fingerprint.rs:92`

- Change: `text: format!("units[{}]", conditions.join(","))` → `text: format!("units[{}]{}", conditions.join(","), filter.unit_type.as_deref().unwrap_or(""))`
- Command (in the copy): `cargo test --release -p kerno-units --all-features --test redaction_units`
- `exit=101`
- Lines as they were printed:

```text
test kerno_db::units_leak_no_content_in_errors_reports_or_observations ... FAILED
test emulated_log::units_leak_no_content_in_errors_reports_or_observations ... FAILED
thread 'kerno_db::units_leak_no_content_in_errors_reports_or_observations' (5321) panicked at crates/kerno-units/tests/redaction_units.rs:440:5:
thread 'emulated_log::units_leak_no_content_in_errors_reports_or_observations' (5320) panicked at crates/kerno-units/tests/redaction_units.rs:440:5:
test result: FAILED. 1 passed; 2 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.26s
```

### Q9: `crates/kerno-units/src/verify.rs:643`

- Change: `self.push(VerifyRule::Payload(e.rule), k, format!("unit {unit} rev {rev}: {}", e.detail));` → `self.push(VerifyRule::Payload(e.rule), k, format!("unit {unit} rev {rev}: {} {:?}", e.detail, stored));`
- Command (in the copy): `cargo test --release -p kerno-units --all-features --test redaction_units`
- `exit=101`
- Lines as they were printed:

```text
test emulated_log::units_leak_no_content_in_errors_reports_or_observations ... FAILED
test kerno_db::units_leak_no_content_in_errors_reports_or_observations ... FAILED
thread 'emulated_log::units_leak_no_content_in_errors_reports_or_observations' (5691) panicked at crates/kerno-units/tests/redaction_units.rs:440:5:
thread 'kerno_db::units_leak_no_content_in_errors_reports_or_observations' (5692) panicked at crates/kerno-units/tests/redaction_units.rs:440:5:
test result: FAILED. 1 passed; 2 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.22s
```

### Q10: `crates/kerno-units/src/verify.rs:643`

- Change: `self.push(VerifyRule::Payload(e.rule), k, format!("unit {unit} rev {rev}: {}", e.detail));` → `let _ = (e, k);`
- Command (in the copy): `cargo test --release -p kerno-units --all-features --test redaction_units`
- `exit=101`
- Lines as they were printed:

```text
test kerno_db::units_leak_no_content_in_errors_reports_or_observations ... FAILED
test emulated_log::units_leak_no_content_in_errors_reports_or_observations ... FAILED
thread 'kerno_db::units_leak_no_content_in_errors_reports_or_observations' (6136) panicked at crates/kerno-units/tests/redaction_units.rs:431:5:
thread 'emulated_log::units_leak_no_content_in_errors_reports_or_observations' (6135) panicked at crates/kerno-units/tests/redaction_units.rs:431:5:
test result: FAILED. 1 passed; 2 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.12s
```

### Q11: `crates/kerno-units/src/observation.rs:489`

- Change: `(level == ObservationLevel::Diagnostic).then(|| shape.text().to_owned())` → `None`
- Command (in the copy): `cargo test --release -p kerno-units --all-features --test redaction_units`
- `exit=101`
- Lines as they were printed:

```text
test emulated_log::units_leak_no_content_in_errors_reports_or_observations ... FAILED
test kerno_db::units_leak_no_content_in_errors_reports_or_observations ... FAILED
thread 'emulated_log::units_leak_no_content_in_errors_reports_or_observations' (6616) panicked at crates/kerno-units/tests/redaction_units.rs:438:5:
thread 'kerno_db::units_leak_no_content_in_errors_reports_or_observations' (6617) panicked at crates/kerno-units/tests/redaction_units.rs:438:5:
test result: FAILED. 1 passed; 2 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.12s
```

### Q12: `crates/kerno-units/src/vector_store.rs:1230`

- Change: `let new = cs.replace_vector(*id, &spec)?;` → `let new = cs.replace_vector(*id, &VectorSpec { vector: spec.vector.iter().rev().copied().collect(), ..spec })?;`
- Command (in the copy): `cargo test --release -p kerno-units --all-features --test derived_measure`
- `exit=101`
- Lines as they were printed:

```text
test quality_over_the_frozen_set ... FAILED
thread 'quality_over_the_frozen_set' (7151) panicked at crates/kerno-units/tests/derived_measure.rs:749:5:
test result: FAILED. 3 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 12.29s
```

### Q13: `crates/kerno-units/tests/derived_measure.rs:718`

- Change: `        changed.len()\n    );\n    put!(out, "P4  rebuilt = built anew` → `        changed.len() + usize::from(E::NAME == "kerno")\n    );\n    put!(out, "P4  rebuilt = built anew`
- Command (in the copy): `cargo test --release -p kerno-units --all-features --test derived_measure`
- `exit=101`
- Lines as they were printed:

```text
test quality_over_the_frozen_set ... FAILED
thread 'quality_over_the_frozen_set' (8016) panicked at crates/kerno-units/tests/derived_measure.rs:747:5:
test result: FAILED. 3 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 12.50s
```

### Q13b: `crates/kerno-storage/src/txn.rs:121`

- Change: `(Some((w, _)), Some((b, _))) => w.as_slice().cmp(b.as_slice()),` → `(Some((w, _)), Some((b, _))) => w.as_slice().cmp(b.as_slice()).then(Ordering::Greater),`
- Command (in the copy): `cargo test --release -p kerno-units --all-features --test derived_measure`
- `exit=0`
- Lines as they were printed:

```text
test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 12.54s
```

### Q13c: `crates/kerno-storage/src/txn.rs:121`

- Change: `(Some((w, _)), Some((b, _))) => w.as_slice().cmp(b.as_slice()),` → `(Some((w, _)), Some((b, _))) => w.as_slice().cmp(b.as_slice()).then(Ordering::Greater),`
- Command (in the copy): `cargo test --release --no-fail-fast -p kerno-storage --all-features --lib --test txn`
- `exit=101`
- Lines as they were printed:

```text
test result: ok. 125 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.88s
test inline::shared_txn::read_your_writes_get_and_scan ... FAILED
test inline::shared_txn::txn_range_delete_then_put ... FAILED
test inline::shared_txn::txn_sees_latest_committed_state ... FAILED
test shared_txn::read_your_writes_get_and_scan ... FAILED
test shared_txn::txn_range_delete_then_put ... FAILED
test shared_txn::txn_sees_latest_committed_state ... FAILED
thread 'inline::shared_txn::read_your_writes_get_and_scan' (20543) panicked at crates/kerno-testkit/src/harness/txn.rs:263:5:
thread 'inline::shared_txn::txn_range_delete_then_put' (20548) panicked at crates/kerno-testkit/src/harness/txn.rs:100:17:
thread 'inline::shared_txn::txn_sees_latest_committed_state' (20549) panicked at crates/kerno-testkit/src/harness/txn.rs:124:5:
thread 'shared_txn::read_your_writes_get_and_scan' (20660) panicked at crates/kerno-testkit/src/harness/txn.rs:263:5:
thread 'shared_txn::txn_range_delete_then_put' (20814) panicked at crates/kerno-testkit/src/harness/txn.rs:100:17:
thread 'shared_txn::txn_sees_latest_committed_state' (20824) panicked at crates/kerno-testkit/src/harness/txn.rs:124:5:
test result: FAILED. 32 passed; 6 failed; 0 ignored; 0 measured; 0 filtered out; finished in 7.04s
```

### Q14: `crates/kerno-storage/src/compaction.rs:799`

- Change: `if cover.covering(it.key()) > it.seq() {` → `if false && cover.covering(it.key()) > it.seq() {`
- Command (in the copy): `cargo test --release -p kerno-storage --all-features --test shared_ranges -- m0_profile::shared_range::space_is_reclaimed_after_range_deletes`
- `exit=101`
- Lines as they were printed:

```text
test m0_profile::shared_range::space_is_reclaimed_after_range_deletes ... FAILED
thread 'm0_profile::shared_range::space_is_reclaimed_after_range_deletes' (10615) panicked at crates/kerno-testkit/src/harness/range.rs:609:9:
test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 71 filtered out; finished in 0.12s
```

### Q15: `crates/kerno-storage/src/deleter.rs:117`

- Change: `match self.vfs.remove_file(&item.path) {` → `match Ok::<(), io::Error>(()) {`
- Command (in the copy): `cargo test --release --no-fail-fast -p kerno-storage --all-features --test shared_ranges --test shared -- space_is_reclaimed`
- `exit=101`
- Lines as they were printed:

```text
test inline::shared_functional::space_is_reclaimed ... FAILED
test shared_functional::space_is_reclaimed ... FAILED
test serial::shared_functional::space_is_reclaimed ... FAILED
test m0_profile::shared_functional::space_is_reclaimed ... FAILED
thread 'inline::shared_functional::space_is_reclaimed' (3924) panicked at crates/kerno-testkit/src/harness/functional.rs:389:13:
cycle 2: 3809172 bytes on disk after refill, first fill used 523798
thread 'shared_functional::space_is_reclaimed' (3927) panicked at crates/kerno-testkit/src/harness/functional.rs:389:13:
cycle 2: 4008815 bytes on disk after refill, first fill used 495580
thread 'serial::shared_functional::space_is_reclaimed' (3926) panicked at crates/kerno-testkit/src/harness/functional.rs:389:13:
cycle 2: 3757624 bytes on disk after refill, first fill used 494050
thread 'm0_profile::shared_functional::space_is_reclaimed' (3925) panicked at crates/kerno-testkit/src/harness/functional.rs:389:13:
cycle 3: 5886018 bytes on disk after refill, first fill used 843755
test result: FAILED. 0 passed; 4 failed; 0 ignored; 0 measured; 92 filtered out; finished in 30.11s
test inline::shared_functional::space_is_reclaimed ... FAILED
test inline::shared_range::space_is_reclaimed_after_range_deletes ... FAILED
test threads::shared_functional::space_is_reclaimed ... FAILED
test m0_profile::shared_range::space_is_reclaimed_after_range_deletes ... FAILED
test threads::shared_range::space_is_reclaimed_after_range_deletes ... FAILED
thread 'inline::shared_functional::space_is_reclaimed' (4766) panicked at crates/kerno-testkit/src/harness/functional.rs:389:13:
cycle 2: 3809172 bytes on disk after refill, first fill used 523798
thread 'inline::shared_range::space_is_reclaimed_after_range_deletes' (4767) panicked at crates/kerno-testkit/src/harness/range.rs:591:13:
cycle 3: 3333751 bytes on disk after refill, first fill used 530874 (bound 3172072, 1801847 value bytes written so far)
thread 'threads::shared_functional::space_is_reclaimed' (4769) panicked at crates/kerno-testkit/src/harness/functional.rs:389:13:
cycle 3: 4897742 bytes on disk after refill, first fill used 522759
thread 'm0_profile::shared_range::space_is_reclaimed_after_range_deletes' (4768) panicked at crates/kerno-testkit/src/harness/range.rs:591:13:
cycle 4: 5140161 bytes on disk after refill, first fill used 806989 (bound 4276532, 2250115 value bytes written so far)
thread 'threads::shared_range::space_is_reclaimed_after_range_deletes' (5477) panicked at crates/kerno-testkit/src/harness/range.rs:591:13:
cycle 4: 4267934 bytes on disk after refill, first fill used 531075 (bound 3172876, 2250115 value bytes written so far)
test result: FAILED. 0 passed; 5 failed; 0 ignored; 0 measured; 67 filtered out; finished in 60.15s
```

### Q16: `crates/kerno-testkit/src/harness/mod.rs:74`

- Change: `    if size <= bound || db.stats().is_empty() {\n        return size;` → `    if size <= bound || db.stats().is_empty() {\n        return 0;`
- Command (in the copy): `cargo test --release -p kerno-testkit --all-features --lib -- space`
- `exit=101`
- Lines as they were printed:

```text
test harness::range::tests::space_check_fails_for_an_engine_that_reclaims_nothing ... FAILED
test harness::range::tests::space_check_fails_for_an_engine_that_keeps_range_deleted_points ... FAILED
thread 'harness::range::tests::space_check_fails_for_an_engine_that_reclaims_nothing' (12219) panicked at crates/kerno-testkit/src/harness/range.rs:797:14:
thread 'harness::range::tests::space_check_fails_for_an_engine_that_keeps_range_deleted_points' (12218) panicked at crates/kerno-testkit/src/harness/range.rs:906:14:
test result: FAILED. 2 passed; 2 failed; 0 ignored; 0 measured; 86 filtered out; finished in 0.16s
```

## P3: plan explanation, evidence, cost and the record of `set_key`

**Method.** Each mutation was applied to a copy of the tree (without `target`, `.git` or `experiments`) with its own build directory (`CARGO_TARGET_DIR` inside the copy), release mode only, one at a time; the file was restored at the end of each one. Order: change the file, run `cargo test --release -p kerno-units --all-features --no-fail-fast --lib --test cache --test cache_planner --test cache_admission --test cache_forget --test cache_reversibility --test cache_sentinels --test observation --test plan`, and note the `test … FAILED` lines as they were printed. The copy and its build directory were deleted at the end. The lines are those of the tree (the copy was identical in `src/`). All 17 mutations came out red (`exit=101`); none survived. M9 was repeated after hardening `the_plan_says_…`: the first time only one test of `cache.rs` turned it red, because the plan test looked at another query than the entry's owner; the record is that of the repetition.

**Test count of `kerno-units`** (`cargo test -p kerno-units --all-features --release -- --list`, lines `: test`): **859 before** (base tree, measured at the start) and **870 after**: +3 from `cache.rs` (`the_created_record_carries_the_evidence_and_the_measured_cost`, `peek_says_what_lookup_would_and_changes_nothing`, `set_key_leaves_one_record_of_the_wipe_in_the_new_log`) and +8 from `cache_planner.rs` (four tests per engine: `relation_evidence_comes_from_the_observer`, `the_plan_says_whether_the_cache_was_consulted_and_whether_learned_state_answers`, `the_plan_does_not_announce_an_entry_that_fails_its_check`, `replacing_the_key_leaves_a_record_of_the_wipe`). They change without changing their number: `observations_never_act_as_authority`, `an_entry_exists_only_with_its_dependencies` and those that now raise the observer before switching the cache on (`cache_admission`, `cache_forget`, `cache_reversibility`, `cache_sentinels`).

**What test each mutation covers.**

| Test (two engines if it touches a database) | Mutations |
|---|---|
| `the_plan_says_whether_the_cache_was_consulted_and_whether_learned_state_answers` | M9, M14, M15, M16 |
| `the_plan_does_not_announce_an_entry_that_fails_its_check` | M13 |
| `observations_never_act_as_authority` (changed) | M1, M2, M3, M4, M6, M8, M17 |
| `relation_evidence_comes_from_the_observer` | M1, M2, M5, M8, M17 |
| `replacing_the_key_leaves_a_record_of_the_wipe` | M11, M12 |
| `an_entry_exists_only_with_its_dependencies` (changed, `cache.rs`) | M7 |
| `the_created_record_carries_the_evidence_and_the_measured_cost` | M8 |
| `peek_says_what_lookup_would_and_changes_nothing` | M9, M10 |
| `set_key_leaves_one_record_of_the_wipe_in_the_new_log` | M11, M12 |
| `no_literal_in_anything_the_cache_path_reports` (changed, `cache_sentinels.rs`) | M1, M3, M17 |
| `a_query_that_comes_twice_is_stored_and_then_served`, `a_deleted_unit_is_not_served_from_the_cache`, `a_cache_in_use_never_changes_what_a_commit_writes` (changed: they raise the observer) | M17 |

**Caveats.** (1) M17 makes the observer never give evidence and so turns red every test that switches the cache on: it shows that the preparation change of those tests is needed, not that each one tests the evidence. (2) M6 and M8 use the figure 1 ns as a "marker": the tests require a cost greater than 100 ns, which a real read exceeds easily in release, but it is a threshold of the test, not a guarantee of the measurement. (3) It was not shown by mutation that `peek` stops being pure (for example, that it counts a read): this is asserted by the comparison of records and counters before and after (`peek_says_…`, `the_plan_says_…`), with no mutation that turns it red. (4) The cost is a single measurement with the system's monotonic clock: no test fixes its value, only that it is measured (greater than 100 ns) and that the plan and the record say the same thing.

## Record P3

### M1: `crates/kerno-units/src/planner.rs:603`

- What it measures: S2: the planner creates the entry even if the observer gives no evidence (constant `{1, 0}` as before)
- Change: `let Some(evidence) = self.observer.evidence_of(&query.normalized()) else { ⏎             return; ⏎         };` → `let evidence = self.observer.evidence_of(&query.normalized()).unwrap_or(crate::cache::Evidence { observations: 1, window: std::time::Duration::ZERO });`
- `exit=101`
- Red tests (6), lines as they were printed:

```text
test emulated_log::observations_never_act_as_authority ... FAILED
test emulated_log::relation_evidence_comes_from_the_observer ... FAILED
test kerno_db::observations_never_act_as_authority ... FAILED
test kerno_db::relation_evidence_comes_from_the_observer ... FAILED
test emulated_log::no_literal_in_anything_the_cache_path_reports ... FAILED
test kerno_db::no_literal_in_anything_the_cache_path_reports ... FAILED
```

### M2: `crates/kerno-units/src/observation.rs:441`

- What it measures: S2: the evidence count does not include the execution that concludes
- Change: `let observations = 1 + u64::from(first.is_some())` → `let observations = 0 + u64::from(first.is_some())`
- `exit=101`
- Red tests (26), lines as they were printed:

```text
test emulated_log::a_slot_that_served_is_replaced_at_once ... FAILED
test emulated_log::a_query_that_comes_twice_is_stored_and_then_served ... FAILED
test kerno_db::a_query_that_comes_twice_is_stored_and_then_served ... FAILED
test emulated_log::a_stream_of_queries_that_never_repeat_copies_nothing ... FAILED
test kerno_db::a_slot_that_served_is_replaced_at_once ... FAILED
test kerno_db::a_stream_of_queries_that_never_repeat_copies_nothing ... FAILED
test emulated_log::a_deleted_unit_is_not_served_from_the_cache ... FAILED
test emulated_log::a_retention_purge_is_not_served_from_the_cache ... FAILED
test kerno_db::a_deleted_unit_is_not_served_from_the_cache ... FAILED
test kerno_db::a_retention_purge_is_not_served_from_the_cache ... FAILED
test emulated_log::every_use_of_the_cache_is_a_transition ... FAILED
test emulated_log::a_failed_check_discards_the_entry_and_the_reference_path_answers ... FAILED
test emulated_log::observations_never_act_as_authority ... FAILED
test emulated_log::relation_evidence_comes_from_the_observer ... FAILED
test emulated_log::replacing_the_key_leaves_a_record_of_the_wipe ... FAILED
test emulated_log::the_plan_says_whether_the_cache_was_consulted_and_whether_learned_state_answers ... FAILED
test kerno_db::a_failed_check_discards_the_entry_and_the_reference_path_answers ... FAILED
test kerno_db::every_use_of_the_cache_is_a_transition ... FAILED
test emulated_log::turning_the_cache_off_restores_the_reference_behaviour ... FAILED
test kerno_db::observations_never_act_as_authority ... FAILED
test kerno_db::replacing_the_key_leaves_a_record_of_the_wipe ... FAILED
test kerno_db::relation_evidence_comes_from_the_observer ... FAILED
test kerno_db::the_plan_says_whether_the_cache_was_consulted_and_whether_learned_state_answers ... FAILED
test kerno_db::turning_the_cache_off_restores_the_reference_behaviour ... FAILED
test kerno_db::no_literal_in_anything_the_cache_path_reports ... FAILED
test emulated_log::no_literal_in_anything_the_cache_path_reports ... FAILED
```

### M3: `crates/kerno-units/src/observation.rs:425`

- What it measures: S2: `evidence_of` gives evidence also at level `Aggregate`
- Change: `if self.level() < ObservationLevel::Fingerprint { ⏎             return None;` → `if self.level() < ObservationLevel::Aggregate { ⏎             return None;`
- `exit=101`
- Red tests (4), lines as they were printed:

```text
test emulated_log::observations_never_act_as_authority ... FAILED
test kerno_db::observations_never_act_as_authority ... FAILED
test emulated_log::no_literal_in_anything_the_cache_path_reports ... FAILED
test kerno_db::no_literal_in_anything_the_cache_path_reports ... FAILED
```

### M4: `crates/kerno-units/src/observation.rs:443`

- What it measures: S2: the evidence window is always zero
- Change: `Duration::from_micros(u64::try_from(now.0.saturating_sub(at)).unwrap_or(0))` → `Duration::from_micros(u64::try_from(now.0.saturating_sub(at)).unwrap_or(0) * 0)`
- `exit=101`
- Red tests (4), lines as they were printed:

```text
test emulated_log::observations_never_act_as_authority ... FAILED
test emulated_log::relation_evidence_comes_from_the_observer ... FAILED
test kerno_db::observations_never_act_as_authority ... FAILED
test kerno_db::relation_evidence_comes_from_the_observer ... FAILED
```

### M5: `crates/kerno-units/src/observation.rs:439`

- What it measures: S2: the evidence counts executions of any form of query
- Change: `.filter(|r| r.fingerprint == fingerprint && r.outcome` → `.filter(|r| r.outcome`
- `exit=101`
- Red tests (2), lines as they were printed:

```text
test emulated_log::relation_evidence_comes_from_the_observer ... FAILED
test kerno_db::relation_evidence_comes_from_the_observer ... FAILED
```

### M6: `crates/kerno-units/src/planner.rs:320`

- What it measures: S3: the measured cost is always 1 ns
- Change: `.unwrap_or(u64::MAX).max(1)` → `.unwrap_or(u64::MAX).min(1)`
- `exit=101`
- Red tests (4), lines as they were printed:

```text
test emulated_log::observations_never_act_as_authority ... FAILED
test emulated_log::relation_evidence_comes_from_the_observer ... FAILED
test kerno_db::observations_never_act_as_authority ... FAILED
test kerno_db::relation_evidence_comes_from_the_observer ... FAILED
```

### M7: `crates/kerno-units/src/cache.rs:353`

- What it measures: S3: `CacheEntry::build` accepts an entry without cost
- Change: `if cost_ns == 0 { ⏎             return Err(CacheError::NoCost); ⏎         }` → `(text removed)`
- `exit=101`
- Red tests (1), lines as they were printed:

```text
test cache::tests::an_entry_exists_only_with_its_dependencies ... FAILED
```

### M8: `crates/kerno-units/src/cache.rs:938`

- What it measures: S3: the `Created` record stores cost 1 instead of the measured one
- Change: `Transition::Created { evidence: entry.evidence, cost_ns: entry.cost_ns }` → `Transition::Created { evidence: entry.evidence, cost_ns: 1 }`
- `exit=101`
- Red tests (10), lines as they were printed:

```text
test cache::tests::every_transition_leaves_a_record ... FAILED
test cache::tests::the_created_record_carries_the_evidence_and_the_measured_cost ... FAILED
test emulated_log::observations_never_act_as_authority ... FAILED
test emulated_log::relation_evidence_comes_from_the_observer ... FAILED
test emulated_log::the_plan_says_whether_the_cache_was_consulted_and_whether_learned_state_answers ... FAILED
test kerno_db::observations_never_act_as_authority ... FAILED
test kerno_db::relation_evidence_comes_from_the_observer ... FAILED
test kerno_db::the_plan_says_whether_the_cache_was_consulted_and_whether_learned_state_answers ... FAILED
test emulated_log::every_g7_transition_leaves_its_record ... FAILED
test kerno_db::every_g7_transition_leaves_its_record ... FAILED
```

### M9: `crates/kerno-units/src/cache.rs:1055`

- What it measures: S1: `peek` does not compare dependencies (it announces a hit with the stale entry)
- Change: `if let Some(dependency) = slot.entry.dependencies.first_change(now) { ⏎             return Peek::Invalidated(dependency); ⏎         }` → `(text removed)`
- `exit=101`
- Red tests (3), lines as they were printed:

```text
test cache::tests::peek_says_what_lookup_would_and_changes_nothing ... FAILED
test emulated_log::the_plan_says_whether_the_cache_was_consulted_and_whether_learned_state_answers ... FAILED
test kerno_db::the_plan_says_whether_the_cache_was_consulted_and_whether_learned_state_answers ... FAILED
```

### M10: `crates/kerno-units/src/cache.rs:1052`

- What it measures: S1: `peek` announces a hit with the entry not activated
- Change: `if !slot.active { ⏎             return Peek::Miss(MissCause::NotActive); ⏎         }` → `(text removed)`
- `exit=101`
- Red tests (1), lines as they were printed:

```text
test cache::tests::peek_says_what_lookup_would_and_changes_nothing ... FAILED
```

### M11: `crates/kerno-units/src/cache.rs:1133`

- What it measures: S4: `set_key` does not leave the record of the wipe
- Change: `st.record(None, Transition::KeyReplaced { entries }, None, now);` → `let _ = (entries, now);`
- `exit=101`
- Red tests (3), lines as they were printed:

```text
test cache::tests::set_key_leaves_one_record_of_the_wipe_in_the_new_log ... FAILED
test emulated_log::replacing_the_key_leaves_a_record_of_the_wipe ... FAILED
test kerno_db::replacing_the_key_leaves_a_record_of_the_wipe ... FAILED
```

### M12: `crates/kerno-units/src/cache.rs:1133`

- What it measures: S4: the `set_key` record says 0 entries
- Change: `st.record(None, Transition::KeyReplaced { entries }, None, now);` → `st.record(None, Transition::KeyReplaced { entries: 0 }, None, now);`
- `exit=101`
- Red tests (3), lines as they were printed:

```text
test cache::tests::set_key_leaves_one_record_of_the_wipe_in_the_new_log ... FAILED
test emulated_log::replacing_the_key_leaves_a_record_of_the_wipe ... FAILED
test kerno_db::replacing_the_key_leaves_a_record_of_the_wipe ... FAILED
```

### M13: `crates/kerno-units/src/planner.rs:464`

- What it measures: S1: the explanation announces a hit even when the rows fail their check
- Change: `Ok(()) => CacheView::Hit { evidence: hit.evidence, cost_ns: hit.cost_ns }, ⏎                 Err(violation) => CacheView::Discrepancy(violation),` → `Ok(()) => CacheView::Hit { evidence: hit.evidence, cost_ns: hit.cost_ns }, ⏎                 Err(_) => CacheView::Hit { evidence: hit.evidence, cost_ns: hit.cost_ns },`
- `exit=101`
- Red tests (2), lines as they were printed:

```text
test emulated_log::the_plan_does_not_announce_an_entry_that_fails_its_check ... FAILED
test kerno_db::the_plan_does_not_announce_an_entry_that_fails_its_check ... FAILED
```

### M14: `crates/kerno-units/src/plan.rs:273`

- What it measures: S1: `consulted()` is true with the cache switched off
- Change: `!matches!(self, CacheView::NotApplicable | CacheView::Off)` → `!matches!(self, CacheView::NotApplicable)`
- `exit=101`
- Red tests (2), lines as they were printed:

```text
test emulated_log::the_plan_says_whether_the_cache_was_consulted_and_whether_learned_state_answers ... FAILED
test kerno_db::the_plan_says_whether_the_cache_was_consulted_and_whether_learned_state_answers ... FAILED
```

### M15: `crates/kerno-units/src/plan.rs:336`

- What it measures: S1: `answering_path()` ignores the cache
- Change: `if self.cache.learned_state_used() { PlanPath::ResultCache } else { self.decision.chosen().path }` → `self.decision.chosen().path`
- `exit=101`
- Red tests (2), lines as they were printed:

```text
test emulated_log::the_plan_says_whether_the_cache_was_consulted_and_whether_learned_state_answers ... FAILED
test kerno_db::the_plan_says_whether_the_cache_was_consulted_and_whether_learned_state_answers ... FAILED
```

### M16: `crates/kerno-units/src/planner.rs:461`

- What it measures: S1: the cause of the cache miss is lost in the explanation
- Change: `Peek::Miss(cause) => CacheView::Miss(cause),` → `Peek::Miss(_) => CacheView::Miss(crate::cache::MissCause::NoEntry),`
- `exit=101`
- Red tests (2), lines as they were printed:

```text
test emulated_log::the_plan_says_whether_the_cache_was_consulted_and_whether_learned_state_answers ... FAILED
test kerno_db::the_plan_says_whether_the_cache_was_consulted_and_whether_learned_state_answers ... FAILED
```

### M17: `crates/kerno-units/src/observation.rs:425`

- What it measures: S2: `evidence_of` never gives evidence
- Change: `if self.level() < ObservationLevel::Fingerprint { ⏎             return None; ⏎         } ⏎         let st = match` → `if self.level() < ObservationLevel::Fingerprint { ⏎             return None; ⏎         } ⏎         return None; ⏎         #[allow(unreachable_code)] ⏎         let st = match`
- `exit=101`
- Red tests (34), lines as they were printed:

```text
test emulated_log::a_slot_that_served_is_replaced_at_once ... FAILED
test emulated_log::a_query_that_comes_twice_is_stored_and_then_served ... FAILED
test emulated_log::a_stream_of_queries_that_never_repeat_copies_nothing ... FAILED
test kerno_db::a_query_that_comes_twice_is_stored_and_then_served ... FAILED
test kerno_db::a_slot_that_served_is_replaced_at_once ... FAILED
test kerno_db::a_stream_of_queries_that_never_repeat_copies_nothing ... FAILED
test emulated_log::a_deleted_unit_is_not_served_from_the_cache ... FAILED
test emulated_log::a_retention_purge_is_not_served_from_the_cache ... FAILED
test kerno_db::a_deleted_unit_is_not_served_from_the_cache ... FAILED
test kerno_db::a_retention_purge_is_not_served_from_the_cache ... FAILED
test emulated_log::every_use_of_the_cache_is_a_transition ... FAILED
test emulated_log::a_failed_check_discards_the_entry_and_the_reference_path_answers ... FAILED
test emulated_log::observations_never_act_as_authority ... FAILED
test emulated_log::cache_on_off_and_invalidated_give_identical_results ... FAILED
test emulated_log::relation_evidence_comes_from_the_observer ... FAILED
test emulated_log::replacing_the_key_leaves_a_record_of_the_wipe ... FAILED
test emulated_log::the_plan_says_whether_the_cache_was_consulted_and_whether_learned_state_answers ... FAILED
test emulated_log::turning_the_cache_off_restores_the_reference_behaviour ... FAILED
test kerno_db::every_use_of_the_cache_is_a_transition ... FAILED
test kerno_db::a_failed_check_discards_the_entry_and_the_reference_path_answers ... FAILED
test kerno_db::relation_evidence_comes_from_the_observer ... FAILED
test kerno_db::observations_never_act_as_authority ... FAILED
test kerno_db::replacing_the_key_leaves_a_record_of_the_wipe ... FAILED
test kerno_db::the_plan_says_whether_the_cache_was_consulted_and_whether_learned_state_answers ... FAILED
test kerno_db::turning_the_cache_off_restores_the_reference_behaviour ... FAILED
test kerno_db::cache_on_off_and_invalidated_give_identical_results ... FAILED
test emulated_log::a_cache_switched_off_after_use_stops_taking_part ... FAILED
test emulated_log::every_operation_on_the_cache_leaves_the_data_as_it_was ... FAILED
test emulated_log::a_cache_in_use_never_changes_what_a_commit_writes ... FAILED
test kerno_db::a_cache_in_use_never_changes_what_a_commit_writes ... FAILED
test kerno_db::a_cache_switched_off_after_use_stops_taking_part ... FAILED
test kerno_db::every_operation_on_the_cache_leaves_the_data_as_it_was ... FAILED
test emulated_log::no_literal_in_anything_the_cache_path_reports ... FAILED
test kerno_db::no_literal_in_anything_the_cache_path_reports ... FAILED
```
