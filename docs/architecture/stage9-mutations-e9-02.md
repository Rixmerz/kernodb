# E9-02: mutation record (T-MU1)

Date: 2026-11-13. Meets §4.4 of the [Stage 9 record](stage9-record.md) for the E9-02 changes after the independent review (decisions D-a to D-f, by express delegation of the owner).

## Method

A single copy of the tree (without `target` or `.git`), with its own build directory, release mode only, deleted when finished. **One** mutation is applied at a time, `cargo test --release -p kerno-units --test public_ids --features test-support --no-fail-fast` is run (mutation M15 lives in `ids.rs` and is run with `--lib`), the result is noted and the file is restored before the next. The line is that of the original file where the changed text begins. "FAILED lines" are those printed by the runner, unedited.

## Results

| Id | File:line | Change | First failing test | Result |
|---|---|---|---|---|
| M01 UNIT-39 token without reverse | `crates/kerno-units/src/verify.rs:684` | `if reverse.is_none() && !self.bad_reverses.contains(u) {` → `if false && !self.bad_reverses.contains(u) {` | `emulated_log::stored::cross_a_token_without_its_reverse` | **fails** (3) |
| M02 UNIT-40 reverse without token | `crates/kerno-units/src/verify.rs:708` | `if owner.is_none() && !self.bad_tokens.contains(t) {` → `if false && !self.bad_tokens.contains(t) {` | `emulated_log::stored::cross_a_pair_naming_a_unit_that_does_not_exist` | **fails** (7) |
| M03 UNIT-41 pair names a missing unit (token side) | `crates/kerno-units/src/verify.rs:697` | `if !exists(self, u) {` → `if false {` | `emulated_log::stored::cross_a_pair_naming_a_unit_that_does_not_exist` | **fails** (5) |
| M04 UNIT-41 deleted unit counts as existing | `crates/kerno-units/src/verify.rs:666` | `s.heads.get(u).is_some_and(|h| h.lifecycle != Lifecycle::Deleted)` → `s.heads.contains_key(u)` | `emulated_log::stored::cross_a_pair_left_on_a_deleted_unit` | **fails** (3) |
| M05 UNIT-42 two tokens for one unit | `crates/kerno-units/src/verify.rs:694` | `if extra {` → `if false {` | `emulated_log::stored::cross_two_tokens_for_one_unit` | **fails** (3) |
| M06 UNIT-43 one token for two units | `crates/kerno-units/src/verify.rs:718` | `if extra {` → `if false {` | `emulated_log::stored::cross_one_token_for_two_units` | **fails** (3) |
| M07 UNIT-44 token records under a mark below 3 | `crates/kerno-units/src/verify.rs:670` | `mark.layer_version < crate::codec::LAYER_VERSION_PUBLIC_IDS` → `mark.layer_version > 99` | `emulated_log::stored::cross_tokens_under_a_mark_below_layer_3` | **fails** (3) |
| M08 delete keeps the mapping | `crates/kerno-units/src/db.rs:1089` | `derived.extend(self.plan_token_removal(plan.rev.unit)?);` → `let _ = self.plan_token_removal(plan.rev.unit)?;` | `emulated_log::deleting_a_unit_removes_its_token_like_one_never_issued` | **fails** (10) |
| M09 import does not restore the token | `crates/kerno-units/src/import.rs:164` | `cs.restore_public_id(u.id, token)?;` → `let _ = (u.id, token);` | `emulated_log::export_carries_tokens_and_import_restores_them` | **fails** (5) |
| M10 import accepts one token for two units | `crates/kerno-units/src/import.rs:432` | `if !seen.insert(bytes) {` → `if !seen.insert(bytes) && false {` | `emulated_log::import_refuses_one_token_for_two_units` | **fails** (3) |
| M11 export writes a wrong token | `crates/kerno-units/src/export.rs:135` | `w.0.extend_from_slice(&token.to_bytes());` → `w.0.extend_from_slice(&[7u8; 16]);` | `emulated_log::export_carries_tokens_and_import_restores_them` | **fails** (7) |
| M12 public_id skips the reverse check | `crates/kerno-units/src/ids.rs:420` | `if self.unit_of(&token)? != Some(unit) {` → `if false {` | `emulated_log::stored::a_reverse_record_without_its_token_is_corruption_in_public_id` | **fails** (6) |
| M13 public_id serves a deleted unit | `crates/kerno-units/src/ids.rs:417` | `if !self.is_live(unit)? {` → `if false {` | `emulated_log::stored::cross_a_pair_left_on_a_deleted_unit` | **fails** (4) |
| M14 resolve serves a deleted unit | `crates/kerno-units/src/ids.rs:436` | `if !self.is_live(unit)? {` → `if false {` | `emulated_log::stored::cross_a_pair_left_on_a_deleted_unit` | **fails** (4) |
| M15 constant source accepted | `crates/kerno-units/src/ids.rs:277` | `if bytes.iter().all(|b| *b == bytes[0]) {` → `if false {` | `ids::tests::short_reads_read_errors_and_constant_tokens_are_refused` | **fails** (2) |
| M16 key length before the value version | `crates/kerno-units/src/ids.rs:261` | `    check_envelope(value)?;` → `    if key.len() != KEY_LEN {` | `emulated_log::stored::verifier_reports_each_malformed_record` | **fails** (4) |
| M17 reserved flags ignored | `crates/kerno-units/src/ids.rs:237` | `if flags != 0 {` → `if false {` | `emulated_log::stored::verifier_reports_each_malformed_record` | **fails** (4) |
| M18 DerivedDb resolve not forwarded | `crates/kerno-units/src/derived_query.rs:37` | `self.db.resolve_public_id(token)` → `Ok(None)` | `emulated_log::derived_db_issues_resolves_and_forgets_tokens` | **fails** (3) |

| M19 UNIT-41 pair names a missing unit (reverse side) | `crates/kerno-units/src/verify.rs:722` | `if owner != Some(u) && !exists(self, u) {` → `if false && !exists(self, u) {` | `emulated_log::stored::cross_a_pair_naming_a_unit_that_does_not_exist` | **fails** (2) |
| M20 record count | `crates/kerno-units/src/verify.rs:474` | `self.counts.public_id_records += 1;` → `self.counts.public_id_records += 0;` | `emulated_log::record_verifier_accepts_tokens` | **fails** (4) |
| M21 envelope_version check | `crates/kerno-units/src/ids.rs:234` | `PUBLIC_ID_RECORD_VERSION => {}` → `_ if version >= PUBLIC_ID_RECORD_VERSION => {}` | `emulated_log::stored::verifier_reports_each_malformed_record` | **fails** (3) |
| M22 export format_version check | `crates/kerno-units/src/import.rs:227` | `if version != EXPORT_VERSION {` → `if false {` | `emulated_log::import_of_a_version_0_export_is_refused` | **fails** (2) |
| M23 layer mark not raised by the first token | `crates/kerno-units/src/db.rs:1547` | `let wants = if self.token_ops > 0 {` → `let wants = if self.token_ops > 99 {` | `emulated_log::mark_is_raised_to_3_by_the_first_token_and_never_lowered` | **fails** (51) |
| M24 layer mark refusal (UNIT-16) | `crates/kerno-units/src/codec.rs:583` | `            v if v <= max => {}` → `            _ => {}` | `emulated_log::layer2_reader_refuses_a_database_with_tokens` | **fails** (5) |
| M25 a unit with no head is live | `crates/kerno-units/src/ids.rs:402` | in `is_live`, the branch without a head `return Ok(false);` → `return Ok(true);` | `emulated_log::stored::cross_a_pair_on_a_unit_with_no_head_is_not_live` | **fails** (2); see note 1 |
| M27 retention drops the mapping of a live unit | `crates/kerno-units/src/retention.rs:652` | after the write of `UnitHead` in `unit_writes`, `writes.push(Write::Delete(<0x71 ‖ unit>))` is added | `emulated_log::retention_leaves_the_mapping_of_a_live_unit` | **fails** (2) |
| M28 token write loses its reverse record (failed-sync test) | `crates/kerno-units/src/ids.rs:358` | after `self.token_ops += 1;`, `self.write(&Write::Delete(unit_key(unit)))?;` is added | `emulated_log::same_transaction_a_failed_sync_leaves_neither_unit_nor_token` | **fails** (63 of 78 in the full run; 2 with the name filter); see note 2 |
| M29 fixture (fixture test) | `crates/kerno-units/tests/vectors/public-ids-v3/TOKEN:1` | the token text of the fixture, `ggext` → `ggaxt` (the fixture is mutated, not the code) | `kerno_db::fixture_directory_opens_resolves_and_verifies` | **fails** (1) |

Notes. (1) M25 had no test that detected it: `is_live` went through `get_unit` and no test put a pair on a unit without a head. `cross_a_pair_on_a_unit_with_no_head_is_not_live` was added (in both engines) and the mutation was run with it. (2) M28 passed the failed-sync test because this only looked at the branches with failure; checks were added to the branch in which the failure does not fire (`public_id`, `resolve_public_id` and the record count) and the mutation was run with them. (3) M29 mutates a test datum and not a line of code: the fixture test is the only one that opens the committed directory. (4) The line numbers of all rows are those of the final tree. M12, M13, M14 and M25 were run again in this round after rewriting `is_live` (ids.rs:420, 417, 436 and 402) and their blocks are the output of that run; M13 and M14 now also detect the unit-without-header test, hence 4 failures instead of 2.

"fails (n)" is the number of failing tests, counting both engines. The 28 mutations (M01 to M25, M27 to M29; there is no M26) were detected; none survived. Each was applied to its own copy of the tree, with its own build directory, and both were deleted when finished.

**Run in this round**: M12, M13, M14 and M25. **Only the line number was checked** (with `grep` over the final tree; a later review found M11 shifted, from `export.rs:133` to `export.rs:135`, and it was corrected; the others match): M01 to M11 and M15 to M24, M27, M28 and M29. Those rows keep the output of their earlier run; none touches code changed afterwards, except `is_live`, which only affects M12 to M14 and M25.

## FAILED lines (up to three per mutation)

### M01 UNIT-39 token without reverse

```
test emulated_log::stored::cross_a_token_without_its_reverse ... FAILED
test kerno_db::stored::cross_a_token_without_its_reverse ... FAILED
test result: FAILED. 74 passed; 2 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.09s
```

### M02 UNIT-40 reverse without token

```
test emulated_log::stored::cross_a_pair_naming_a_unit_that_does_not_exist ... FAILED
test emulated_log::stored::cross_a_reverse_without_its_token ... FAILED
test emulated_log::stored::cross_two_tokens_for_one_unit ... FAILED
test result: FAILED. 70 passed; 6 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.10s
```

### M03 UNIT-41 pair names a missing unit (token side)

```
test emulated_log::stored::cross_a_pair_naming_a_unit_that_does_not_exist ... FAILED
test emulated_log::stored::cross_a_pair_left_on_a_deleted_unit ... FAILED
test kerno_db::stored::cross_a_pair_left_on_a_deleted_unit ... FAILED
test result: FAILED. 72 passed; 4 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.10s
```

### M04 UNIT-41 deleted unit counts as existing

```
test emulated_log::stored::cross_a_pair_left_on_a_deleted_unit ... FAILED
test kerno_db::stored::cross_a_pair_left_on_a_deleted_unit ... FAILED
test result: FAILED. 74 passed; 2 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.10s
```

### M05 UNIT-42 two tokens for one unit

```
test emulated_log::stored::cross_two_tokens_for_one_unit ... FAILED
test kerno_db::stored::cross_two_tokens_for_one_unit ... FAILED
test result: FAILED. 74 passed; 2 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.10s
```

### M06 UNIT-43 one token for two units

```
test emulated_log::stored::cross_one_token_for_two_units ... FAILED
test kerno_db::stored::cross_one_token_for_two_units ... FAILED
test result: FAILED. 74 passed; 2 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.09s
```

### M07 UNIT-44 token records under a mark below 3

```
test emulated_log::stored::cross_tokens_under_a_mark_below_layer_3 ... FAILED
test kerno_db::stored::cross_tokens_under_a_mark_below_layer_3 ... FAILED
test result: FAILED. 74 passed; 2 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.10s
```

### M08 delete keeps the mapping

```
test emulated_log::deleting_a_unit_removes_its_token_like_one_never_issued ... FAILED
test emulated_log::derived_db_issues_resolves_and_forgets_tokens ... FAILED
test emulated_log::fixture_directory_opens_resolves_and_verifies ... FAILED
test result: FAILED. 67 passed; 9 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.10s
```

### M09 import does not restore the token

```
test emulated_log::export_carries_tokens_and_import_restores_them ... FAILED
test emulated_log::export_import_export_keeps_every_token_across_two_hops ... FAILED
test kerno_db::export_carries_tokens_and_import_restores_them ... FAILED
test result: FAILED. 72 passed; 4 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.09s
```

### M10 import accepts one token for two units

```
test emulated_log::import_refuses_one_token_for_two_units ... FAILED
test kerno_db::import_refuses_one_token_for_two_units ... FAILED
test result: FAILED. 74 passed; 2 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.10s
```

### M11 export writes a wrong token

```
test emulated_log::export_carries_tokens_and_import_restores_them ... FAILED
test emulated_log::export_import_export_keeps_every_token_across_two_hops ... FAILED
test emulated_log::import_refuses_one_token_for_two_units ... FAILED
test result: FAILED. 70 passed; 6 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.09s
```

### M12 public_id skips the reverse check

```
test emulated_log::stored::a_reverse_record_without_its_token_is_corruption_in_public_id ... FAILED
test emulated_log::stored::disagreeing_records_are_corruption ... FAILED
test emulated_log::stored::error_texts_never_pair_a_token_with_a_unit_id ... FAILED
test kerno_db::stored::a_reverse_record_without_its_token_is_corruption_in_public_id ... FAILED
test kerno_db::stored::error_texts_never_pair_a_token_with_a_unit_id ... FAILED
test kerno_db::stored::disagreeing_records_are_corruption ... FAILED
test result: FAILED. 72 passed; 6 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.10s
```

### M13 public_id serves a deleted unit

```
test emulated_log::stored::cross_a_pair_left_on_a_deleted_unit ... FAILED
test emulated_log::stored::cross_a_pair_on_a_unit_with_no_head_is_not_live ... FAILED
test kerno_db::stored::cross_a_pair_left_on_a_deleted_unit ... FAILED
test kerno_db::stored::cross_a_pair_on_a_unit_with_no_head_is_not_live ... FAILED
test result: FAILED. 74 passed; 4 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.11s
```

### M14 resolve serves a deleted unit

```
test emulated_log::stored::cross_a_pair_left_on_a_deleted_unit ... FAILED
test emulated_log::stored::cross_a_pair_on_a_unit_with_no_head_is_not_live ... FAILED
test kerno_db::stored::cross_a_pair_left_on_a_deleted_unit ... FAILED
test kerno_db::stored::cross_a_pair_on_a_unit_with_no_head_is_not_live ... FAILED
test result: FAILED. 74 passed; 4 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.10s
```

### M15 constant source accepted

```
test ids::tests::short_reads_read_errors_and_constant_tokens_are_refused ... FAILED
test result: FAILED. 64 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.12s
```

### M16 key length before the value version

```
test emulated_log::stored::verifier_reports_each_malformed_record ... FAILED
test kerno_db::stored::verifier_reports_each_malformed_record ... FAILED
test vectors_of_the_document_match_the_codec_byte_for_byte ... FAILED
test result: FAILED. 73 passed; 3 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.11s
```

### M17 reserved flags ignored

```
test emulated_log::stored::verifier_reports_each_malformed_record ... FAILED
test kerno_db::stored::verifier_reports_each_malformed_record ... FAILED
test vectors_of_the_document_match_the_codec_byte_for_byte ... FAILED
test result: FAILED. 73 passed; 3 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.09s
```

### M18 DerivedDb resolve not forwarded

```
test emulated_log::derived_db_issues_resolves_and_forgets_tokens ... FAILED
test kerno_db::derived_db_issues_resolves_and_forgets_tokens ... FAILED
test result: FAILED. 74 passed; 2 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.09s
```

### M19 UNIT-41 pair names a missing unit (reverse side)

```
test emulated_log::stored::cross_a_pair_naming_a_unit_that_does_not_exist ... FAILED
test kerno_db::stored::cross_a_pair_naming_a_unit_that_does_not_exist ... FAILED
test result: FAILED. 76 passed; 2 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.10s
```

### M20 record count

```
test emulated_log::record_verifier_accepts_tokens ... FAILED
test emulated_log::stored::cross_counts_the_records ... FAILED
test kerno_db::record_verifier_accepts_tokens ... FAILED
test result: FAILED. 74 passed; 4 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.10s
```

### M21 envelope_version check

```
test emulated_log::stored::verifier_reports_each_malformed_record ... FAILED
test vectors_of_the_document_match_the_codec_byte_for_byte ... FAILED
test kerno_db::stored::verifier_reports_each_malformed_record ... FAILED
test result: FAILED. 75 passed; 3 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.10s
```

### M22 export format_version check

```
test emulated_log::import_of_a_version_0_export_is_refused ... FAILED
test kerno_db::import_of_a_version_0_export_is_refused ... FAILED
test result: FAILED. 76 passed; 2 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.09s
```

### M23 layer mark not raised by the first token

```
test emulated_log::deleting_a_unit_removes_its_token_like_one_never_issued ... FAILED
test emulated_log::derived_db_issues_resolves_and_forgets_tokens ... FAILED
test emulated_log::export_carries_tokens_and_import_restores_them ... FAILED
test result: FAILED. 27 passed; 51 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.12s
```

### M24 layer mark refusal (UNIT-16)

```
test emulated_log::fixture_directory_is_refused_by_a_layer2_reader ... FAILED
test emulated_log::layer2_reader_refuses_a_database_with_tokens ... FAILED
test kerno_db::fixture_directory_is_refused_by_a_layer2_reader ... FAILED
test result: FAILED. 73 passed; 5 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.10s
```

### M25 a unit with no head is live

```
test emulated_log::stored::cross_a_pair_on_a_unit_with_no_head_is_not_live ... FAILED
test kerno_db::stored::cross_a_pair_on_a_unit_with_no_head_is_not_live ... FAILED
test result: FAILED. 76 passed; 2 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.09s
```

### M27 retention drops the mapping of a live unit

```
test emulated_log::retention_leaves_the_mapping_of_a_live_unit ... FAILED
test kerno_db::retention_leaves_the_mapping_of_a_live_unit ... FAILED
test result: FAILED. 76 passed; 2 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.18s
```

### M28 token write loses its reverse record (with the filter `same_transaction_a_failed_sync`)

```
test emulated_log::same_transaction_a_failed_sync_leaves_neither_unit_nor_token ... FAILED
test kerno_db::same_transaction_a_failed_sync_leaves_neither_unit_nor_token ... FAILED
test result: FAILED. 0 passed; 2 failed; 0 ignored; 0 measured; 77 filtered out; finished in 0.09s
(full run, last line:)
test result: FAILED. 15 passed; 63 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.11s
```

### M29 fixture (with the filter `fixture`)

```
test kerno_db::fixture_directory_opens_resolves_and_verifies ... FAILED
test result: FAILED. 4 passed; 1 failed; 1 ignored; 0 measured; 73 filtered out; finished in 0.09s
```
