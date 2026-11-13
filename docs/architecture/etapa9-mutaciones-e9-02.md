# E9-02: registro de mutaciones (T-MU1)

Fecha: 2026-11-13. Cumple el §4.4 del [acta de la Etapa 9](etapa9-acta.md) para los cambios de E9-02 tras la revisión independiente (decisiones D-a a D-f, por delegación expresa del propietario).

## Método

Una sola copia del árbol (sin `target` ni `.git`), con su propio directorio de compilación, solo en modo release, borrada al terminar. Se aplica **una** mutación cada vez, se ejecuta `cargo test --release -p kerno-units --test public_ids --features test-support --no-fail-fast` (la mutación M15 vive en `ids.rs` y se ejecuta con `--lib`), se anota el resultado y se restaura el fichero antes de la siguiente. La línea es la del fichero original donde empieza el texto cambiado. «Líneas FAILED» son las impresas por el ejecutor, sin editar.

## Resultados

| Id | Fichero:línea | Cambio | Primera prueba que falla | Resultado |
|---|---|---|---|---|
| M01 UNIT-39 token without reverse | `crates/kerno-units/src/verify.rs:684` | `if reverse.is_none() && !self.bad_reverses.contains(u) {` → `if false && !self.bad_reverses.contains(u) {` | `emulated_log::stored::cross_a_token_without_its_reverse` | **falla** (3) |
| M02 UNIT-40 reverse without token | `crates/kerno-units/src/verify.rs:708` | `if owner.is_none() && !self.bad_tokens.contains(t) {` → `if false && !self.bad_tokens.contains(t) {` | `emulated_log::stored::cross_a_pair_naming_a_unit_that_does_not_exist` | **falla** (7) |
| M03 UNIT-41 pair names a missing unit (token side) | `crates/kerno-units/src/verify.rs:697` | `if !exists(self, u) {` → `if false {` | `emulated_log::stored::cross_a_pair_naming_a_unit_that_does_not_exist` | **falla** (5) |
| M04 UNIT-41 deleted unit counts as existing | `crates/kerno-units/src/verify.rs:666` | `s.heads.get(u).is_some_and(|h| h.lifecycle != Lifecycle::Deleted)` → `s.heads.contains_key(u)` | `emulated_log::stored::cross_a_pair_left_on_a_deleted_unit` | **falla** (3) |
| M05 UNIT-42 two tokens for one unit | `crates/kerno-units/src/verify.rs:694` | `if extra {` → `if false {` | `emulated_log::stored::cross_two_tokens_for_one_unit` | **falla** (3) |
| M06 UNIT-43 one token for two units | `crates/kerno-units/src/verify.rs:718` | `if extra {` → `if false {` | `emulated_log::stored::cross_one_token_for_two_units` | **falla** (3) |
| M07 UNIT-44 token records under a mark below 3 | `crates/kerno-units/src/verify.rs:670` | `mark.layer_version < crate::codec::LAYER_VERSION_PUBLIC_IDS` → `mark.layer_version > 99` | `emulated_log::stored::cross_tokens_under_a_mark_below_layer_3` | **falla** (3) |
| M08 delete keeps the mapping | `crates/kerno-units/src/db.rs:1089` | `derived.extend(self.plan_token_removal(plan.rev.unit)?);` → `let _ = self.plan_token_removal(plan.rev.unit)?;` | `emulated_log::deleting_a_unit_removes_its_token_like_one_never_issued` | **falla** (10) |
| M09 import does not restore the token | `crates/kerno-units/src/import.rs:164` | `cs.restore_public_id(u.id, token)?;` → `let _ = (u.id, token);` | `emulated_log::export_carries_tokens_and_import_restores_them` | **falla** (5) |
| M10 import accepts one token for two units | `crates/kerno-units/src/import.rs:432` | `if !seen.insert(bytes) {` → `if !seen.insert(bytes) && false {` | `emulated_log::import_refuses_one_token_for_two_units` | **falla** (3) |
| M11 export writes a wrong token | `crates/kerno-units/src/export.rs:133` | `w.0.extend_from_slice(&token.to_bytes());` → `w.0.extend_from_slice(&[7u8; 16]);` | `emulated_log::export_carries_tokens_and_import_restores_them` | **falla** (7) |
| M12 public_id skips the reverse check | `crates/kerno-units/src/ids.rs:411` | `if self.unit_of(&token)? != Some(unit) {` → `if false {` | `emulated_log::stored::a_reverse_record_without_its_token_is_corruption_in_public_id` | **falla** (7) |
| M13 public_id serves a deleted unit | `crates/kerno-units/src/ids.rs:408` | `if !self.is_live(unit)? {` → `if false {` | `emulated_log::stored::cross_a_pair_left_on_a_deleted_unit` | **falla** (3) |
| M14 resolve serves a deleted unit | `crates/kerno-units/src/ids.rs:427` | `if !self.is_live(unit)? {` → `if false {` | `emulated_log::stored::cross_a_pair_left_on_a_deleted_unit` | **falla** (3) |
| M15 constant source accepted | `crates/kerno-units/src/ids.rs:277` | `if bytes.iter().all(|b| *b == bytes[0]) {` → `if false {` | `ids::tests::short_reads_read_errors_and_constant_tokens_are_refused` | **falla** (2) |
| M16 key length before the value version | `crates/kerno-units/src/ids.rs:261` | `    check_envelope(value)?;` → `    if key.len() != KEY_LEN {` | `emulated_log::stored::verifier_reports_each_malformed_record` | **falla** (4) |
| M17 reserved flags ignored | `crates/kerno-units/src/ids.rs:237` | `if flags != 0 {` → `if false {` | `emulated_log::stored::verifier_reports_each_malformed_record` | **falla** (4) |
| M18 DerivedDb resolve not forwarded | `crates/kerno-units/src/derived_query.rs:37` | `self.db.resolve_public_id(token)` → `Ok(None)` | `emulated_log::derived_db_issues_resolves_and_forgets_tokens` | **falla** (3) |

«falla (n)» es el número de pruebas fallidas, contando los dos motores. Las 18 mutaciones fueron detectadas; ninguna sobrevivió. Cada una se aplicó sobre la copia y se restauró antes de la siguiente; la copia y su directorio de compilación se borraron al terminar.

**No ejecutado** (se dice, no se afirma): mutaciones sobre las pruebas del fixture (`fixture_*`), sobre la prueba de sincronización fallida (`same_transaction_a_failed_sync_*`), sobre el rechazo de una exportación de versión 0 y sobre `retention_leaves_the_mapping_of_a_live_unit`. Esas pruebas pasan, pero no se las vio fallar bajo una mutación en este registro.

## Líneas FAILED (hasta tres por mutación)

### M01 UNIT-39 token without reverse

```
test emulated_log::stored::cross_a_token_without_its_reverse ... FAILED
test kerno_db::stored::cross_a_token_without_its_reverse ... FAILED
test result: FAILED. 74 passed; 2 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.09s
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
test result: FAILED. 74 passed; 2 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.10s
```

### M05 UNIT-42 two tokens for one unit

```
test emulated_log::stored::cross_two_tokens_for_one_unit ... FAILED
test kerno_db::stored::cross_two_tokens_for_one_unit ... FAILED
test result: FAILED. 74 passed; 2 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.10s
test result: FAILED. 74 passed; 2 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.10s
```

### M06 UNIT-43 one token for two units

```
test emulated_log::stored::cross_one_token_for_two_units ... FAILED
test kerno_db::stored::cross_one_token_for_two_units ... FAILED
test result: FAILED. 74 passed; 2 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.09s
test result: FAILED. 74 passed; 2 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.09s
```

### M07 UNIT-44 token records under a mark below 3

```
test emulated_log::stored::cross_tokens_under_a_mark_below_layer_3 ... FAILED
test kerno_db::stored::cross_tokens_under_a_mark_below_layer_3 ... FAILED
test result: FAILED. 74 passed; 2 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.10s
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
test result: FAILED. 70 passed; 6 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.10s
```

### M13 public_id serves a deleted unit

```
test emulated_log::stored::cross_a_pair_left_on_a_deleted_unit ... FAILED
test kerno_db::stored::cross_a_pair_left_on_a_deleted_unit ... FAILED
test result: FAILED. 74 passed; 2 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.10s
test result: FAILED. 74 passed; 2 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.10s
```

### M14 resolve serves a deleted unit

```
test emulated_log::stored::cross_a_pair_left_on_a_deleted_unit ... FAILED
test kerno_db::stored::cross_a_pair_left_on_a_deleted_unit ... FAILED
test result: FAILED. 74 passed; 2 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.09s
test result: FAILED. 74 passed; 2 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.09s
```

### M15 constant source accepted

```
test ids::tests::short_reads_read_errors_and_constant_tokens_are_refused ... FAILED
test result: FAILED. 64 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.12s
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
test result: FAILED. 74 passed; 2 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.09s
```

