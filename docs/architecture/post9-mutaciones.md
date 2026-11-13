# Mutaciones posteriores a la Etapa 9

## P1: contadores de adyacencia (K1)

**Método.** Cada mutación se aplicó a una copia del árbol (sin `target`, `.git` ni `experiments`) con su propio directorio de compilación (`CARGO_TARGET_DIR` dentro de la copia), solo en modo release, una cada vez; después de cada una se restauró el fichero. Orden: cambiar el fichero, ejecutar `cargo test --release -p kerno-units --all-features --no-fail-fast` con los ficheros de prueba indicados, anotar las líneas `test … FAILED` tal como se imprimieron. La copia y su directorio de compilación se borraron al terminar. Las líneas citadas son las del árbol (la copia era idéntica).

**Ficheros de prueba ejecutados.** M1 a M16: `rel_counts`, `relations`, `assoc_indexed`. M17 y M19: `verify`, `unit_db`. M18: `assoc_reference`. Todas las mutaciones salieron en rojo (`exit=101`); ninguna sobrevivió.

**Conteo de pruebas de `kerno-units`** (`cargo test -p kerno-units --all-features --release -- --list`, líneas `: test`): **822 antes** (con el código de P1 pero sin pruebas nuevas, medido al empezar) y **853 después**: +31 de `rel_counts.rs` (30 se ejecutan y 1 `#[ignore]` que regenera el fixture; 14 pruebas por motor en dos motores, más el vector del documento, el manifiesto del fixture y el regenerador). El resto de pruebas cambiadas (`relations.rs`, `assoc_indexed.rs`, `assoc_reference.rs`, `verify.rs`, `unit_db.rs`) no cambian de número; `completeness_is_not_a_query_time_check` se renombró.

**Qué prueba cubre cada mutación.** Cada prueba nueva o cambiada falla al menos bajo una:

| Prueba | Mutación |
|---|---|
| `one_entry_lost_{small,large}_store`, `one_of_several_entries_lost`, `a_count_that_disagrees_is_an_error` | M1, M4, M5, M11 |
| `one_adjacency_entry_missing_{small,large}_store`, `verifier_reports_index_damage` (relations.rs) | M1, M11 |
| `false_index_answers_…`, `a_relation_lost_with_both_entries_is_caught_by_its_counts` (assoc_indexed.rs) | M1 |
| `the_reference_path_never_reads_the_adjacency_index` (assoc_reference.rs) | M18 |
| `record_layout_is_pinned` | M2, M12 |
| `counts_follow_create_and_delete` | M2, M3, M14 |
| `an_aborted_change_set_leaves_no_count` | M15 (mutación de la propia prueba) |
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

**Salvedades.** (1) M15 muta la prueba, no `src/`: la prueba de aborto descansa en la reversión del motor y ninguna mutación razonable de `src/` la pone en rojo. (2) M16 altera un byte del `MANIFEST` comprometido, no código. (3) Algunas mutaciones (M6, M12, M13) ponen en rojo además pruebas ajenas a P1 (`create_and_read`, `differential_seeded`…), lo que es esperable porque cambian el comportamiento de toda relación; se listan tal como salieron. (4) M1 y M2 se ejecutaron con la compilación en frío. (5) La comprobación completa (`cargo test -p kerno-core -p kerno-storage -p kerno-testkit -p kerno-units --all-features --release --no-fail-fast`) terminó con `exit=0` y 101 líneas `test result` sin fallos, tras corregir las pruebas que fijaban el comportamiento anterior.

## Registro

### M1: `crates/kerno-units/src/adj_count.rs:324`

- Cambio: `'        if got != *want {'` → `'        if false && got != *want {'`
- `exit=101`
- Pruebas rojas (20), líneas tal como se imprimieron:

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

### M2: `crates/kerno-units/src/db.rs:1473`

- Cambio: `'        self.adjust_count(to, Direction::In, relation_type, 1)?;\n'` → `''`
- `exit=101`
- Pruebas rojas (22), líneas tal como se imprimieron:

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

### M3: `crates/kerno-units/src/db.rs:1500`

- Cambio: `'        self.adjust_count(r.from, Direction::Out, &r.relation_type, -1)?;\n'` → `''`
- `exit=101`
- Pruebas rojas (12), líneas tal como se imprimieron:

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

- Cambio: `'    let (start, end) = count_range(unit, dir);\n    scan_all('` → `'    let (start, end) = count_range(unit, dir);\n    if true {\n        return Ok(out);\n    }\n    scan_all('`
- `exit=101`
- Pruebas rojas (16), líneas tal como se imprimieron:

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

- Cambio: `'            if got != *want {\n                self.cross(\n                    CrossRule::Unit45'` → `'            if false && got != *want {\n                self.cross(\n                    CrossRule::Unit45'`
- `exit=101`
- Pruebas rojas (8), líneas tal como se imprimieron:

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

### M6: `crates/kerno-units/src/db.rs:1569`

- Cambio: `'        let wants = if self.count_ops > 0 {'` → `'        let wants = if false && self.count_ops > 0 {'`
- `exit=101`
- Pruebas rojas (52), líneas tal como se imprimieron:

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

- Cambio: `'    let Some((start, end)) = Key::adjacency_range(adjacency_tag(dir), unit, Some(relation_type)) else {\n        return Ok(0);\n    };'` → `'    let Some((start, end)) = Key::adjacency_range(adjacency_tag(dir), unit, Some(relation_type)) else {\n        return Ok(0);\n    };\n    if true {\n        return Ok(0);\n    }'`
- `exit=101`
- Pruebas rojas (4), líneas tal como se imprimieron:

```text
test emulated_log::deleting_a_legacy_relation_counts_what_is_left ... FAILED
test emulated_log::legacy_relations_work_and_get_a_count_when_next_changed ... FAILED
test kerno::deleting_a_legacy_relation_counts_what_is_left ... FAILED
test kerno::legacy_relations_work_and_get_a_count_when_next_changed ... FAILED
```

### M8: `crates/kerno-units/src/import.rs:205`

- Cambio: `'    cs.adjust_count(r.from, crate::types::Direction::Out, ty, 1)?;\n    cs.adjust_count(r.to, crate::types::Direction::In, ty, 1)?;\n'` → `''`
- `exit=101`
- Pruebas rojas (2), líneas tal como se imprimieron:

```text
test emulated_log::export_import_keeps_the_counts ... FAILED
test kerno::export_import_keeps_the_counts ... FAILED
```

### M9: `crates/kerno-units/src/adj_count.rs:153`

- Cambio: `'    if count == 0 {'` → `'    if false && count == 0 {'`
- `exit=101`
- Pruebas rojas (3), líneas tal como se imprimieron:

```text
test emulated_log::verifier_rules_of_the_count_records ... FAILED
test kerno::verifier_rules_of_the_count_records ... FAILED
test vectors_of_the_document_match_the_codec_byte_for_byte ... FAILED
```

### M10: `crates/kerno-units/src/verify.rs:922`

- Cambio: `'            && mark.layer_version < crate::codec::LAYER_VERSION_REL_COUNTS'` → `'            && mark.layer_version < 0'`
- `exit=101`
- Pruebas rojas (2), líneas tal como se imprimieron:

```text
test emulated_log::verifier_rules_of_the_count_records ... FAILED
test kerno::verifier_rules_of_the_count_records ... FAILED
```

### M11: `crates/kerno-units/src/planner.rs:830`

- Cambio: `'                Err(e) if is_corruption(&e) => {\n                    fallback = Some(Fallback { abandoned: chosen, cause: FallbackCause::Corrupt });\n                }\n                Err(e) => return Err(e),\n            }\n        }\n        Ok(RelationsOutcome {'` → `'                Err(e) if false && is_corruption(&e) => {\n                    fallback = Some(Fallback { abandoned: chosen, cause: FallbackCause::Corrupt });\n                }\n                Err(e) => return Err(e),\n            }\n        }\n        Ok(RelationsOutcome {'`
- `exit=101`
- Pruebas rojas (10), líneas tal como se imprimieron:

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

- Cambio: `'        Direction::In => 1,\n    }\n}\n\npub(crate) fn dir_of'` → `'        Direction::In => 0,\n    }\n}\n\npub(crate) fn dir_of'`
- `exit=101`
- Pruebas rojas (51), líneas tal como se imprimieron:

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

- Cambio: `'    for (t, want) in expected {\n'` → `'    if expected.is_empty() {\n        return Err(crate::db::corruption("mutation".into()));\n    }\n    for (t, want) in expected {\n'`
- `exit=101`
- Pruebas rojas (26), líneas tal como se imprimieron:

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

- Cambio: `'pub const LAYER_VERSION_REL_COUNTS: u32 = 4;'` → `'pub const LAYER_VERSION_REL_COUNTS: u32 = 3;'`
- `exit=101`
- Pruebas rojas (17), líneas tal como se imprimieron:

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

- Cambio: `'    drop(cs);\n    assert!(count_records(&db).is_empty());'` → `'    drop(cs);\n    assert!(!count_records(&db).is_empty());'`
- `exit=101`
- Pruebas rojas (2), líneas tal como se imprimieron:

```text
test emulated_log::an_aborted_change_set_leaves_no_count ... FAILED
test kerno::an_aborted_change_set_leaves_no_count ... FAILED
```

### M16: `crates/kerno-units/tests/vectors/rel-counts-v4/MANIFEST:1`

- Cambio: `None` → `None`
- `exit=101`
- Pruebas rojas (1), líneas tal como se imprimieron:

```text
test fixture_manifest_lists_the_committed_files ... FAILED
```

### M17 (extra): `crates/kerno-units/src/codec.rs:60`

- Cambio: `'pub const MAX_LAYER_VERSION: u32 = LAYER_VERSION_REL_COUNTS;'` → `'pub const MAX_LAYER_VERSION: u32 = LAYER_VERSION_PUBLIC_IDS;'`
- `exit=101`
- Pruebas rojas (4), líneas tal como se imprimieron:

```text
test emulated_log::open_and_mark ... FAILED
test kerno::open_and_mark ... FAILED
test emulated_log::damage_detected ... FAILED
test kerno::damage_detected ... FAILED
```

### M18: `crates/kerno-units/src/adj_count.rs:324`

- Cambio: `'        if got != *want {'` → `'        if false && got != *want {'`
- `exit=101`
- Pruebas rojas (2), líneas tal como se imprimieron:

```text
test emulated_log::the_reference_path_never_reads_the_adjacency_index ... FAILED
test kerno::the_reference_path_never_reads_the_adjacency_index ... FAILED
```

### M19: `crates/kerno-units/src/codec.rs:60`

- Cambio: `'pub const MAX_LAYER_VERSION: u32 = LAYER_VERSION_REL_COUNTS;'` → `'pub const MAX_LAYER_VERSION: u32 = 5;'`
- `exit=101`
- Pruebas rojas (7), líneas tal como se imprimieron:

```text
test emulated_log::open_and_mark ... FAILED
test kerno::open_and_mark ... FAILED
test emulated_log::verify_writes_nothing ... FAILED
test emulated_log::damage_detected ... FAILED
test kerno::verify_writes_nothing ... FAILED
test kerno::damage_detected ... FAILED
test kerno_read_only_verify_untouched ... FAILED
```

## P2: pruebas que faltaban y un texto

Paquete P2 (puntos 3, 4, 5, 6, 10 y 11 de [etapa9-cierre.md](etapa9-cierre.md) §5; estado en [post9-pendientes.md](post9-pendientes.md)).

**Método.** Igual que P1: una copia del árbol (sin `target`, `.git` ni `experiments`) con su propio `CARGO_TARGET_DIR`, solo release (con `CARGO_PROFILE_RELEASE_DEBUG=0` para ahorrar disco), una mutación cada vez, el fichero restaurado después de cada una y la copia borrada al terminar. Cada mutación sustituye un texto que aparece una sola vez en su fichero; la línea es la de ese texto en el árbol. Q2, Q3, Q4, Q7, Q13c y Q15 se ejecutaron con `--no-fail-fast` porque su comando abarca varios binarios de prueba y, sin esa opción, `cargo` se detiene en el primero que falla. Todas las mutaciones de las pruebas nuevas o cambiadas salieron en rojo (`exit=101`); la única en verde es Q13b, que no es una mutación de una prueba de P2 sino un candidato descartado (ver Salvedades).

**Conteo de pruebas** (`cargo test -p kerno-core -p kerno-storage -p kerno-testkit -p kerno-units --all-features --release -- --list`, líneas `: test`): **1 798** en el árbol base y **1 804** con P2 (+6): `compact_asks_for_the_revision_ranges`, `compact_refuses_a_poisoned_handle_emulated_log`, `compact_refuses_a_poisoned_handle_kerno_db` (`reclaim.rs`), `db::accessor_tests::compact_covers_exactly_the_revision_tags_kerno_db` (`db.rs`), `emulated_log_registry_is_locked_while_the_engine_commits` y `kerno_db_registry_is_locked_while_the_engine_commits` (`derived_lock_order.rs`, nuevo). Las pruebas cambiadas (`consolidation.rs`, `derived_stale.rs`, `redaction_units.rs`, y las dos pruebas de espacio del arnés) cambian de cuerpo, no de número.

**Qué prueba cubre cada mutación.**

| Prueba | Mutación |
|---|---|
| `compact_refuses_a_poisoned_handle_{emulated_log,kerno_db}` (`reclaim.rs`) | Q1 |
| `compact_asks_for_the_revision_ranges` (`reclaim.rs`), `compact_covers_exactly_the_revision_tags_kerno_db` (`db.rs`) | Q2, Q3, Q4 |
| `{emulated_log,kerno_db}_registry_is_locked_while_the_engine_commits` (`derived_lock_order.rs`) | Q5, Q6 |
| `errors_say_what_happened_without_content` (`consolidation.rs`), `every_error_says_what_it_is_without_naming_content` (`derived_stale.rs`) | Q7 |
| `units_leak_no_content_in_errors_reports_or_observations` (`redaction_units.rs`), aserción de fuga | Q8, Q9 |
| la misma, aserción `payload_findings >= 2` | Q10 |
| la misma, aserción `shape_texts >= 1` | Q11 |
| `quality_over_the_frozen_set` (`derived_measure.rs`), P4 (`derived_measure.rs:749`) | Q12 |
| la misma, informe idéntico entre motores (`derived_measure.rs:747`) | Q13 |
| `m0_profile::shared_range::space_is_reclaimed_after_range_deletes`, aserción de claves | Q14 |
| `space_is_reclaimed_after_range_deletes` y `space_is_reclaimed`, todos los perfiles, aserción de tamaño tras la espera de `settled_size` | Q15 |
| `space_check_fails_for_an_engine_that_reclaims_nothing` y `space_check_fails_for_an_engine_that_keeps_range_deleted_points` (pruebas propias del arnés), la medida única de `settled_size` | Q16 |

**Salvedades.**

1. **Q13 muta la prueba, no `src/`.** Todo el código de `kerno-units` que recorre esta medición es genérico sobre el motor y no tiene ninguna rama que dependa de él (fuera de `compact()`, que la medición no llama, no lee ni el nombre ni las estadísticas del motor); una mutación en `kerno-units` se aplica igual a los dos motores y deja los dos informes iguales, como muestra Q12 (falla P4 en la línea 749, no la igualdad de la 747). Una diferencia solo puede venir de que un motor se aparte del contrato. Se probó un candidato así en `kerno-storage`, Q13b (en una transacción, a igualdad de clave gana la base y no lo escrito por la propia transacción): la medición sigue en verde porque ninguna de sus transacciones lee por rango una clave que ella misma escribió, y las pruebas de conformidad de transacciones del motor la ven (Q13c, 6 fallos). Por eso la aserción 747 se muestra con Q13, una cifra del informe que depende del motor.
2. Q5 mueve la toma del bloqueo después del commit; ninguna mutación muestra que el bloqueo siga tomado durante el marcado (ver el punto 4 de pendientes). Q6 muta el gancho `hook_registry_locked` (solo con `test-support`), no código de producción: muestra que la prueba lee la respuesta del gancho.
3. Q14 deja en rojo la aserción de claves (`range.rs:609`, 0,12 s) y no llega a la de tamaño. La aserción de tamaño y la espera de `settled_size` las ejercita Q15, que tarda unos 30 s por prueba porque la espera se agota con un motor que no reclama; Q15 es la que muestra que la espera no oculta un motor que no reclama.
4. Q2 a Q4 son tres formas de ensanchar el rango (empezar un byte antes, acabar un byte después, sin fin). La prueba de `db.rs` solo corre en `kerno_db` (los contadores de tumbas son suyos); la de `reclaim.rs` ve el rango exacto en el motor de referencia, detrás del envoltorio que implementa `Reclaim`.

## Registro P2

### Q1: `crates/kerno-units/src/retention.rs:671`

- Cambio: `        self.check_poisoned()?;\n        let before = stored_bytes` → `        let before = stored_bytes`
- Comando (en la copia): `cargo test --release -p kerno-units --all-features --test reclaim`
- `exit=101`
- Líneas tal como se imprimieron:

```text
test compact_refuses_a_poisoned_handle_emulated_log ... FAILED
test compact_refuses_a_poisoned_handle_kerno_db ... FAILED
thread 'compact_refuses_a_poisoned_handle_emulated_log' (2372) panicked at crates/kerno-units/tests/reclaim.rs:443:5:
thread 'compact_refuses_a_poisoned_handle_kerno_db' (2373) panicked at crates/kerno-units/tests/reclaim.rs:443:5:
test result: FAILED. 11 passed; 2 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.07s
```

### Q2: `crates/kerno-units/src/retention.rs:675`

- Cambio: `Bound::Included(&[TAG_REV_META]),` → `Bound::Included(&[TAG_REV_META - 1]),`
- Comando (en la copia): `cargo test --release --no-fail-fast -p kerno-units --all-features --lib --test reclaim`
- `exit=101`
- Líneas tal como se imprimieron:

```text
test db::accessor_tests::compact_covers_exactly_the_revision_tags_kerno_db ... FAILED
thread 'db::accessor_tests::compact_covers_exactly_the_revision_tags_kerno_db' (12880) panicked at crates/kerno-units/src/db.rs:1815:13:
test result: FAILED. 65 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.13s
test compact_asks_for_the_revision_ranges ... FAILED
thread 'compact_asks_for_the_revision_ranges' (12998) panicked at crates/kerno-units/tests/reclaim.rs:427:5:
test result: FAILED. 12 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.07s
```

### Q3: `crates/kerno-units/src/retention.rs:676`

- Cambio: `Bound::Excluded(&[TAG_REV_PAYLOAD + 1]),` → `Bound::Excluded(&[TAG_REV_PAYLOAD + 2]),`
- Comando (en la copia): `cargo test --release --no-fail-fast -p kerno-units --all-features --lib --test reclaim`
- `exit=101`
- Líneas tal como se imprimieron:

```text
test db::accessor_tests::compact_covers_exactly_the_revision_tags_kerno_db ... FAILED
thread 'db::accessor_tests::compact_covers_exactly_the_revision_tags_kerno_db' (13311) panicked at crates/kerno-units/src/db.rs:1815:13:
test result: FAILED. 65 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.13s
test compact_asks_for_the_revision_ranges ... FAILED
thread 'compact_asks_for_the_revision_ranges' (13385) panicked at crates/kerno-units/tests/reclaim.rs:427:5:
test result: FAILED. 12 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.07s
```

### Q4: `crates/kerno-units/src/retention.rs:676`

- Cambio: `Bound::Excluded(&[TAG_REV_PAYLOAD + 1]),` → `Bound::Unbounded,`
- Comando (en la copia): `cargo test --release --no-fail-fast -p kerno-units --all-features --lib --test reclaim`
- `exit=101`
- Líneas tal como se imprimieron:

```text
test db::accessor_tests::compact_covers_exactly_the_revision_tags_kerno_db ... FAILED
thread 'db::accessor_tests::compact_covers_exactly_the_revision_tags_kerno_db' (13743) panicked at crates/kerno-units/src/db.rs:1815:13:
test result: FAILED. 65 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.12s
test compact_asks_for_the_revision_ranges ... FAILED
thread 'compact_asks_for_the_revision_ranges' (13817) panicked at crates/kerno-units/tests/reclaim.rs:427:5:
test result: FAILED. 12 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.07s
```

### Q5: `crates/kerno-units/src/derived_stale.rs:568`

- Cambio: `        let mut inner = self.owner.lock();\n        let info = self.cs.commit()?;` → `        let info = self.cs.commit()?;\n        let mut inner = self.owner.lock();`
- Comando (en la copia): `cargo test --release -p kerno-units --all-features --test derived_lock_order`
- `exit=101`
- Líneas tal como se imprimieron:

```text
test emulated_log_registry_is_locked_while_the_engine_commits ... FAILED
test kerno_db_registry_is_locked_while_the_engine_commits ... FAILED
thread 'emulated_log_registry_is_locked_while_the_engine_commits' (3825) panicked at crates/kerno-units/tests/derived_lock_order.rs:147:5:
thread 'kerno_db_registry_is_locked_while_the_engine_commits' (3826) panicked at crates/kerno-units/tests/derived_lock_order.rs:147:5:
test result: FAILED. 0 passed; 2 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.09s
```

### Q6: `crates/kerno-units/src/derived_stale.rs:387`

- Cambio: `        matches!(self.inner.try_lock(), Err(std::sync::TryLockError::WouldBlock))` → `        !matches!(self.inner.try_lock(), Err(std::sync::TryLockError::WouldBlock))`
- Comando (en la copia): `cargo test --release -p kerno-units --all-features --test derived_lock_order`
- `exit=101`
- Líneas tal como se imprimieron:

```text
test emulated_log_registry_is_locked_while_the_engine_commits ... FAILED
test kerno_db_registry_is_locked_while_the_engine_commits ... FAILED
thread 'emulated_log_registry_is_locked_while_the_engine_commits' (4223) panicked at crates/kerno-units/tests/derived_lock_order.rs:136:5:
thread 'kerno_db_registry_is_locked_while_the_engine_commits' (4224) panicked at crates/kerno-units/tests/derived_lock_order.rs:136:5:
test result: FAILED. 0 passed; 2 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.10s
```

### Q7: `crates/kerno-units/src/derived_stale.rs:124`

- Cambio: `is valid any more", IdHex(id))` → `is valid any more", IdHex(&DerivedId([0; 32])))`
- Comando (en la copia): `cargo test --release --no-fail-fast -p kerno-units --all-features --test consolidation --test derived_stale`
- `exit=101`
- Líneas tal como se imprimieron:

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

- Cambio: `text: format!("units[{}]", conditions.join(","))` → `text: format!("units[{}]{}", conditions.join(","), filter.unit_type.as_deref().unwrap_or(""))`
- Comando (en la copia): `cargo test --release -p kerno-units --all-features --test redaction_units`
- `exit=101`
- Líneas tal como se imprimieron:

```text
test kerno_db::units_leak_no_content_in_errors_reports_or_observations ... FAILED
test emulated_log::units_leak_no_content_in_errors_reports_or_observations ... FAILED
thread 'kerno_db::units_leak_no_content_in_errors_reports_or_observations' (5321) panicked at crates/kerno-units/tests/redaction_units.rs:440:5:
thread 'emulated_log::units_leak_no_content_in_errors_reports_or_observations' (5320) panicked at crates/kerno-units/tests/redaction_units.rs:440:5:
test result: FAILED. 1 passed; 2 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.26s
```

### Q9: `crates/kerno-units/src/verify.rs:643`

- Cambio: `self.push(VerifyRule::Payload(e.rule), k, format!("unit {unit} rev {rev}: {}", e.detail));` → `self.push(VerifyRule::Payload(e.rule), k, format!("unit {unit} rev {rev}: {} {:?}", e.detail, stored));`
- Comando (en la copia): `cargo test --release -p kerno-units --all-features --test redaction_units`
- `exit=101`
- Líneas tal como se imprimieron:

```text
test emulated_log::units_leak_no_content_in_errors_reports_or_observations ... FAILED
test kerno_db::units_leak_no_content_in_errors_reports_or_observations ... FAILED
thread 'emulated_log::units_leak_no_content_in_errors_reports_or_observations' (5691) panicked at crates/kerno-units/tests/redaction_units.rs:440:5:
thread 'kerno_db::units_leak_no_content_in_errors_reports_or_observations' (5692) panicked at crates/kerno-units/tests/redaction_units.rs:440:5:
test result: FAILED. 1 passed; 2 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.22s
```

### Q10: `crates/kerno-units/src/verify.rs:643`

- Cambio: `self.push(VerifyRule::Payload(e.rule), k, format!("unit {unit} rev {rev}: {}", e.detail));` → `let _ = (e, k);`
- Comando (en la copia): `cargo test --release -p kerno-units --all-features --test redaction_units`
- `exit=101`
- Líneas tal como se imprimieron:

```text
test kerno_db::units_leak_no_content_in_errors_reports_or_observations ... FAILED
test emulated_log::units_leak_no_content_in_errors_reports_or_observations ... FAILED
thread 'kerno_db::units_leak_no_content_in_errors_reports_or_observations' (6136) panicked at crates/kerno-units/tests/redaction_units.rs:431:5:
thread 'emulated_log::units_leak_no_content_in_errors_reports_or_observations' (6135) panicked at crates/kerno-units/tests/redaction_units.rs:431:5:
test result: FAILED. 1 passed; 2 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.12s
```

### Q11: `crates/kerno-units/src/observation.rs:489`

- Cambio: `(level == ObservationLevel::Diagnostic).then(|| shape.text().to_owned())` → `None`
- Comando (en la copia): `cargo test --release -p kerno-units --all-features --test redaction_units`
- `exit=101`
- Líneas tal como se imprimieron:

```text
test emulated_log::units_leak_no_content_in_errors_reports_or_observations ... FAILED
test kerno_db::units_leak_no_content_in_errors_reports_or_observations ... FAILED
thread 'emulated_log::units_leak_no_content_in_errors_reports_or_observations' (6616) panicked at crates/kerno-units/tests/redaction_units.rs:438:5:
thread 'kerno_db::units_leak_no_content_in_errors_reports_or_observations' (6617) panicked at crates/kerno-units/tests/redaction_units.rs:438:5:
test result: FAILED. 1 passed; 2 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.12s
```

### Q12: `crates/kerno-units/src/vector_store.rs:1230`

- Cambio: `let new = cs.replace_vector(*id, &spec)?;` → `let new = cs.replace_vector(*id, &VectorSpec { vector: spec.vector.iter().rev().copied().collect(), ..spec })?;`
- Comando (en la copia): `cargo test --release -p kerno-units --all-features --test derived_measure`
- `exit=101`
- Líneas tal como se imprimieron:

```text
test quality_over_the_frozen_set ... FAILED
thread 'quality_over_the_frozen_set' (7151) panicked at crates/kerno-units/tests/derived_measure.rs:749:5:
test result: FAILED. 3 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 12.29s
```

### Q13: `crates/kerno-units/tests/derived_measure.rs:718`

- Cambio: `        changed.len()\n    );\n    put!(out, "P4  rebuilt = built anew` → `        changed.len() + usize::from(E::NAME == "kerno")\n    );\n    put!(out, "P4  rebuilt = built anew`
- Comando (en la copia): `cargo test --release -p kerno-units --all-features --test derived_measure`
- `exit=101`
- Líneas tal como se imprimieron:

```text
test quality_over_the_frozen_set ... FAILED
thread 'quality_over_the_frozen_set' (8016) panicked at crates/kerno-units/tests/derived_measure.rs:747:5:
test result: FAILED. 3 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 12.50s
```

### Q13b: `crates/kerno-storage/src/txn.rs:121`

- Cambio: `(Some((w, _)), Some((b, _))) => w.as_slice().cmp(b.as_slice()),` → `(Some((w, _)), Some((b, _))) => w.as_slice().cmp(b.as_slice()).then(Ordering::Greater),`
- Comando (en la copia): `cargo test --release -p kerno-units --all-features --test derived_measure`
- `exit=0`
- Líneas tal como se imprimieron:

```text
test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 12.54s
```

### Q13c: `crates/kerno-storage/src/txn.rs:121`

- Cambio: `(Some((w, _)), Some((b, _))) => w.as_slice().cmp(b.as_slice()),` → `(Some((w, _)), Some((b, _))) => w.as_slice().cmp(b.as_slice()).then(Ordering::Greater),`
- Comando (en la copia): `cargo test --release --no-fail-fast -p kerno-storage --all-features --lib --test txn`
- `exit=101`
- Líneas tal como se imprimieron:

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

- Cambio: `if cover.covering(it.key()) > it.seq() {` → `if false && cover.covering(it.key()) > it.seq() {`
- Comando (en la copia): `cargo test --release -p kerno-storage --all-features --test shared_ranges -- m0_profile::shared_range::space_is_reclaimed_after_range_deletes`
- `exit=101`
- Líneas tal como se imprimieron:

```text
test m0_profile::shared_range::space_is_reclaimed_after_range_deletes ... FAILED
thread 'm0_profile::shared_range::space_is_reclaimed_after_range_deletes' (10615) panicked at crates/kerno-testkit/src/harness/range.rs:609:9:
test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 71 filtered out; finished in 0.12s
```

### Q15: `crates/kerno-storage/src/deleter.rs:117`

- Cambio: `match self.vfs.remove_file(&item.path) {` → `match Ok::<(), io::Error>(()) {`
- Comando (en la copia): `cargo test --release --no-fail-fast -p kerno-storage --all-features --test shared_ranges --test shared -- space_is_reclaimed`
- `exit=101`
- Líneas tal como se imprimieron:

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

- Cambio: `    if size <= bound || db.stats().is_empty() {\n        return size;` → `    if size <= bound || db.stats().is_empty() {\n        return 0;`
- Comando (en la copia): `cargo test --release -p kerno-testkit --all-features --lib -- space`
- `exit=101`
- Líneas tal como se imprimieron:

```text
test harness::range::tests::space_check_fails_for_an_engine_that_reclaims_nothing ... FAILED
test harness::range::tests::space_check_fails_for_an_engine_that_keeps_range_deleted_points ... FAILED
thread 'harness::range::tests::space_check_fails_for_an_engine_that_reclaims_nothing' (12219) panicked at crates/kerno-testkit/src/harness/range.rs:797:14:
thread 'harness::range::tests::space_check_fails_for_an_engine_that_keeps_range_deleted_points' (12218) panicked at crates/kerno-testkit/src/harness/range.rs:906:14:
test result: FAILED. 2 passed; 2 failed; 0 ignored; 0 measured; 86 filtered out; finished in 0.16s
```

## P3: explicación del plan, evidencia, coste y registro de `set_key`

**Método.** Cada mutación se aplicó a una copia del árbol (sin `target`, `.git` ni `experiments`) con su propio directorio de compilación (`CARGO_TARGET_DIR` dentro de la copia), solo en modo release, una cada vez; el fichero se restauraba al terminar cada una. Orden: cambiar el fichero, ejecutar `cargo test --release -p kerno-units --all-features --no-fail-fast --lib --test cache --test cache_planner --test cache_admission --test cache_forget --test cache_reversibility --test cache_sentinels --test observation --test plan`, anotar las líneas `test … FAILED` tal como se imprimieron. La copia y su directorio de compilación se borraron al terminar. Las líneas son las del árbol (la copia era idéntica en `src/`). Las 17 mutaciones salieron en rojo (`exit=101`); ninguna sobrevivió. M9 se repitió después de endurecer `the_plan_says_…`: la primera vez solo la ponía en rojo una prueba de `cache.rs`, porque la prueba del plan miraba otra consulta que la dueña de la entrada; el registro es el de la repetición.

**Conteo de pruebas de `kerno-units`** (`cargo test -p kerno-units --all-features --release -- --list`, líneas `: test`): **859 antes** (árbol base, medido al empezar) y **870 después**: +3 de `cache.rs` (`the_created_record_carries_the_evidence_and_the_measured_cost`, `peek_says_what_lookup_would_and_changes_nothing`, `set_key_leaves_one_record_of_the_wipe_in_the_new_log`) y +8 de `cache_planner.rs` (cuatro pruebas por motor: `relation_evidence_comes_from_the_observer`, `the_plan_says_whether_the_cache_was_consulted_and_whether_learned_state_answers`, `the_plan_does_not_announce_an_entry_that_fails_its_check`, `replacing_the_key_leaves_a_record_of_the_wipe`). Cambian sin cambiar de número: `observations_never_act_as_authority`, `an_entry_exists_only_with_its_dependencies` y las que ahora suben el observador antes de encender la caché (`cache_admission`, `cache_forget`, `cache_reversibility`, `cache_sentinels`).

**Qué prueba cubre cada mutación.**

| Prueba (dos motores si toca base de datos) | Mutaciones |
|---|---|
| `the_plan_says_whether_the_cache_was_consulted_and_whether_learned_state_answers` | M9, M14, M15, M16 |
| `the_plan_does_not_announce_an_entry_that_fails_its_check` | M13 |
| `observations_never_act_as_authority` (cambiada) | M1, M2, M3, M4, M6, M8, M17 |
| `relation_evidence_comes_from_the_observer` | M1, M2, M5, M8, M17 |
| `replacing_the_key_leaves_a_record_of_the_wipe` | M11, M12 |
| `an_entry_exists_only_with_its_dependencies` (cambiada, `cache.rs`) | M7 |
| `the_created_record_carries_the_evidence_and_the_measured_cost` | M8 |
| `peek_says_what_lookup_would_and_changes_nothing` | M9, M10 |
| `set_key_leaves_one_record_of_the_wipe_in_the_new_log` | M11, M12 |
| `no_literal_in_anything_the_cache_path_reports` (cambiada, `cache_sentinels.rs`) | M1, M3, M17 |
| `a_query_that_comes_twice_is_stored_and_then_served`, `a_deleted_unit_is_not_served_from_the_cache`, `a_cache_in_use_never_changes_what_a_commit_writes` (cambiadas: suben el observador) | M17 |

**Salvedades.** (1) M17 hace que el observador no dé nunca evidencia y por eso pone en rojo todas las pruebas que encienden la caché: es lo que muestra que el cambio de preparación de esas pruebas hace falta, no que cada una pruebe la evidencia. (2) M6 y M8 usan la cifra 1 ns como «marcador»: las pruebas exigen un coste mayor que 100 ns, que una lectura real supera de sobra en release, pero es un umbral de la prueba, no una garantía de la medida. (3) No se probó por mutación que `peek` deje de ser puro (por ejemplo, que cuente una lectura): lo afirma la comparación de registros y contadores antes y después (`peek_says_…`, `the_plan_says_…`), sin mutación que la ponga en rojo. (4) El coste es una sola medida con el reloj monotónico del sistema: no hay prueba que fije su valor, solo que es medido (mayor que 100 ns) y que el plan y el registro dicen lo mismo.

## Registro P3

### M1: `crates/kerno-units/src/planner.rs:603`

- Qué mide: S2: el planificador crea la entrada aunque el observador no dé evidencia (constante `{1, 0}` como antes)
- Cambio: `let Some(evidence) = self.observer.evidence_of(&query.normalized()) else { ⏎             return; ⏎         };` → `let evidence = self.observer.evidence_of(&query.normalized()).unwrap_or(crate::cache::Evidence { observations: 1, window: std::time::Duration::ZERO });`
- `exit=101`
- Pruebas rojas (6), líneas tal como se imprimieron:

```text
test emulated_log::observations_never_act_as_authority ... FAILED
test emulated_log::relation_evidence_comes_from_the_observer ... FAILED
test kerno_db::observations_never_act_as_authority ... FAILED
test kerno_db::relation_evidence_comes_from_the_observer ... FAILED
test emulated_log::no_literal_in_anything_the_cache_path_reports ... FAILED
test kerno_db::no_literal_in_anything_the_cache_path_reports ... FAILED
```

### M2: `crates/kerno-units/src/observation.rs:441`

- Qué mide: S2: la cuenta de evidencia no incluye la ejecución que concluye
- Cambio: `let observations = 1 + u64::from(first.is_some())` → `let observations = 0 + u64::from(first.is_some())`
- `exit=101`
- Pruebas rojas (26), líneas tal como se imprimieron:

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

- Qué mide: S2: `evidence_of` da evidencia también a nivel `Aggregate`
- Cambio: `if self.level() < ObservationLevel::Fingerprint { ⏎             return None;` → `if self.level() < ObservationLevel::Aggregate { ⏎             return None;`
- `exit=101`
- Pruebas rojas (4), líneas tal como se imprimieron:

```text
test emulated_log::observations_never_act_as_authority ... FAILED
test kerno_db::observations_never_act_as_authority ... FAILED
test emulated_log::no_literal_in_anything_the_cache_path_reports ... FAILED
test kerno_db::no_literal_in_anything_the_cache_path_reports ... FAILED
```

### M4: `crates/kerno-units/src/observation.rs:443`

- Qué mide: S2: la ventana de la evidencia es siempre cero
- Cambio: `Duration::from_micros(u64::try_from(now.0.saturating_sub(at)).unwrap_or(0))` → `Duration::from_micros(u64::try_from(now.0.saturating_sub(at)).unwrap_or(0) * 0)`
- `exit=101`
- Pruebas rojas (4), líneas tal como se imprimieron:

```text
test emulated_log::observations_never_act_as_authority ... FAILED
test emulated_log::relation_evidence_comes_from_the_observer ... FAILED
test kerno_db::observations_never_act_as_authority ... FAILED
test kerno_db::relation_evidence_comes_from_the_observer ... FAILED
```

### M5: `crates/kerno-units/src/observation.rs:439`

- Qué mide: S2: la evidencia cuenta ejecuciones de cualquier forma de consulta
- Cambio: `.filter(|r| r.fingerprint == fingerprint && r.outcome` → `.filter(|r| r.outcome`
- `exit=101`
- Pruebas rojas (2), líneas tal como se imprimieron:

```text
test emulated_log::relation_evidence_comes_from_the_observer ... FAILED
test kerno_db::relation_evidence_comes_from_the_observer ... FAILED
```

### M6: `crates/kerno-units/src/planner.rs:320`

- Qué mide: S3: el coste medido es siempre 1 ns
- Cambio: `.unwrap_or(u64::MAX).max(1)` → `.unwrap_or(u64::MAX).min(1)`
- `exit=101`
- Pruebas rojas (4), líneas tal como se imprimieron:

```text
test emulated_log::observations_never_act_as_authority ... FAILED
test emulated_log::relation_evidence_comes_from_the_observer ... FAILED
test kerno_db::observations_never_act_as_authority ... FAILED
test kerno_db::relation_evidence_comes_from_the_observer ... FAILED
```

### M7: `crates/kerno-units/src/cache.rs:353`

- Qué mide: S3: `CacheEntry::build` acepta una entrada sin coste
- Cambio: `if cost_ns == 0 { ⏎             return Err(CacheError::NoCost); ⏎         }` → `(texto eliminado)`
- `exit=101`
- Pruebas rojas (1), líneas tal como se imprimieron:

```text
test cache::tests::an_entry_exists_only_with_its_dependencies ... FAILED
```

### M8: `crates/kerno-units/src/cache.rs:938`

- Qué mide: S3: el registro `Created` guarda coste 1 en vez del medido
- Cambio: `Transition::Created { evidence: entry.evidence, cost_ns: entry.cost_ns }` → `Transition::Created { evidence: entry.evidence, cost_ns: 1 }`
- `exit=101`
- Pruebas rojas (10), líneas tal como se imprimieron:

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

- Qué mide: S1: `peek` no compara dependencias (anuncia un acierto con la entrada obsoleta)
- Cambio: `if let Some(dependency) = slot.entry.dependencies.first_change(now) { ⏎             return Peek::Invalidated(dependency); ⏎         }` → `(texto eliminado)`
- `exit=101`
- Pruebas rojas (3), líneas tal como se imprimieron:

```text
test cache::tests::peek_says_what_lookup_would_and_changes_nothing ... FAILED
test emulated_log::the_plan_says_whether_the_cache_was_consulted_and_whether_learned_state_answers ... FAILED
test kerno_db::the_plan_says_whether_the_cache_was_consulted_and_whether_learned_state_answers ... FAILED
```

### M10: `crates/kerno-units/src/cache.rs:1052`

- Qué mide: S1: `peek` anuncia un acierto con la entrada sin activar
- Cambio: `if !slot.active { ⏎             return Peek::Miss(MissCause::NotActive); ⏎         }` → `(texto eliminado)`
- `exit=101`
- Pruebas rojas (1), líneas tal como se imprimieron:

```text
test cache::tests::peek_says_what_lookup_would_and_changes_nothing ... FAILED
```

### M11: `crates/kerno-units/src/cache.rs:1133`

- Qué mide: S4: `set_key` no deja el registro del borrado
- Cambio: `st.record(None, Transition::KeyReplaced { entries }, None, now);` → `let _ = (entries, now);`
- `exit=101`
- Pruebas rojas (3), líneas tal como se imprimieron:

```text
test cache::tests::set_key_leaves_one_record_of_the_wipe_in_the_new_log ... FAILED
test emulated_log::replacing_the_key_leaves_a_record_of_the_wipe ... FAILED
test kerno_db::replacing_the_key_leaves_a_record_of_the_wipe ... FAILED
```

### M12: `crates/kerno-units/src/cache.rs:1133`

- Qué mide: S4: el registro de `set_key` dice 0 entradas
- Cambio: `st.record(None, Transition::KeyReplaced { entries }, None, now);` → `st.record(None, Transition::KeyReplaced { entries: 0 }, None, now);`
- `exit=101`
- Pruebas rojas (3), líneas tal como se imprimieron:

```text
test cache::tests::set_key_leaves_one_record_of_the_wipe_in_the_new_log ... FAILED
test emulated_log::replacing_the_key_leaves_a_record_of_the_wipe ... FAILED
test kerno_db::replacing_the_key_leaves_a_record_of_the_wipe ... FAILED
```

### M13: `crates/kerno-units/src/planner.rs:464`

- Qué mide: S1: la explicación anuncia un acierto aunque las filas fallen su comprobación
- Cambio: `Ok(()) => CacheView::Hit { evidence: hit.evidence, cost_ns: hit.cost_ns }, ⏎                 Err(violation) => CacheView::Discrepancy(violation),` → `Ok(()) => CacheView::Hit { evidence: hit.evidence, cost_ns: hit.cost_ns }, ⏎                 Err(_) => CacheView::Hit { evidence: hit.evidence, cost_ns: hit.cost_ns },`
- `exit=101`
- Pruebas rojas (2), líneas tal como se imprimieron:

```text
test emulated_log::the_plan_does_not_announce_an_entry_that_fails_its_check ... FAILED
test kerno_db::the_plan_does_not_announce_an_entry_that_fails_its_check ... FAILED
```

### M14: `crates/kerno-units/src/plan.rs:273`

- Qué mide: S1: `consulted()` es verdadero con la caché apagada
- Cambio: `!matches!(self, CacheView::NotApplicable | CacheView::Off)` → `!matches!(self, CacheView::NotApplicable)`
- `exit=101`
- Pruebas rojas (2), líneas tal como se imprimieron:

```text
test emulated_log::the_plan_says_whether_the_cache_was_consulted_and_whether_learned_state_answers ... FAILED
test kerno_db::the_plan_says_whether_the_cache_was_consulted_and_whether_learned_state_answers ... FAILED
```

### M15: `crates/kerno-units/src/plan.rs:336`

- Qué mide: S1: `answering_path()` ignora la caché
- Cambio: `if self.cache.learned_state_used() { PlanPath::ResultCache } else { self.decision.chosen().path }` → `self.decision.chosen().path`
- `exit=101`
- Pruebas rojas (2), líneas tal como se imprimieron:

```text
test emulated_log::the_plan_says_whether_the_cache_was_consulted_and_whether_learned_state_answers ... FAILED
test kerno_db::the_plan_says_whether_the_cache_was_consulted_and_whether_learned_state_answers ... FAILED
```

### M16: `crates/kerno-units/src/planner.rs:461`

- Qué mide: S1: la causa del fallo de caché se pierde en la explicación
- Cambio: `Peek::Miss(cause) => CacheView::Miss(cause),` → `Peek::Miss(_) => CacheView::Miss(crate::cache::MissCause::NoEntry),`
- `exit=101`
- Pruebas rojas (2), líneas tal como se imprimieron:

```text
test emulated_log::the_plan_says_whether_the_cache_was_consulted_and_whether_learned_state_answers ... FAILED
test kerno_db::the_plan_says_whether_the_cache_was_consulted_and_whether_learned_state_answers ... FAILED
```

### M17: `crates/kerno-units/src/observation.rs:425`

- Qué mide: S2: `evidence_of` no da nunca evidencia
- Cambio: `if self.level() < ObservationLevel::Fingerprint { ⏎             return None; ⏎         } ⏎         let st = match` → `if self.level() < ObservationLevel::Fingerprint { ⏎             return None; ⏎         } ⏎         return None; ⏎         #[allow(unreachable_code)] ⏎         let st = match`
- `exit=101`
- Pruebas rojas (34), líneas tal como se imprimieron:

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
