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
| `a_refused_create_leaves_no_count` | M20 (corrección posterior de K1, ver más abajo) |
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

### M2: `crates/kerno-units/src/db.rs:1487`

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

### M3: `crates/kerno-units/src/db.rs:1508`

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

### M6: `crates/kerno-units/src/db.rs:1572`

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

### M8: `crates/kerno-units/src/import.rs:213`

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

## Corrección posterior de K1: el rechazo del `encode` no deja contadores

Una revisión independiente halló que `create_relation` ajustaba los dos contadores antes de que `record.encode()` validara la procedencia: si el `encode` rechazaba la relación, el contador ajustado quedaba en el change set abierto y un `commit` posterior lo escribía, y `relations_of` daba `Corruption` y `verify_units` UNIT-45 sobre un almacén sin daño. La corrección construye y codifica todas las escrituras antes de tocar los contadores (`db.rs:1473-1487`); `delete_relation` hace lo mismo con las claves (`db.rs:1503-1509`). La prueba `a_refused_create_leaves_no_count` (rel_counts.rs, dos motores: `Imported` con fuente vacía rechazada, luego una relación válida y `commit`) se vio roja antes de la corrección en los dos motores (`Corruption` en `relations_of`).

Método como arriba (copia del árbol, `CARGO_TARGET_DIR` propio, release, una mutación), con `rel_counts`.

### M20: `crates/kerno-units/src/db.rs:1486`

- Cambio: las dos líneas `adjust_count(from, …, 1)` y `adjust_count(to, …, 1)` se mueven antes del bloque que codifica las escrituras (el orden anterior a la corrección).
- `exit=101`
- Pruebas rojas (2), líneas tal como se imprimieron:

```text
test emulated_log::a_refused_create_leaves_no_count ... FAILED
test kerno::a_refused_create_leaves_no_count ... FAILED
```

Los demás caminos que ajustan contadores se auditaron: `delete_relation` (tras la corrección sus únicos errores previos a la escritura son lecturas y claves; los de `adjust_count` solo se dan con un registro de contador dañado o con un fallo del motor, que deja el change set roto), importación (`import.rs:213`: tenía la misma forma —codificaba el registro tras ajustar los contadores—, pero el change set es privado de `replay` y se abandona sin `commit` ante cualquier error, así que el daño no era alcanzable y no admite prueba; se reordenó igualmente para que construya y codifique antes de tocar los contadores), borrado y olvido de unidades (no ajustan contadores: `ensure_no_relations` exige que no haya relaciones), y los lotes (`DerivedChangeSet::create_relation` delega en `create_relation`).

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
