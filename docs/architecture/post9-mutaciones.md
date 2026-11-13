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
