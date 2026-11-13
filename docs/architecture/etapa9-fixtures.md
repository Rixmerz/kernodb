# E9-01: fixtures de versiones previas

Paquete E9-01 de la Etapa 9 (`etapa9-acta.md`, §3; decisión D11, opción (a); fila F-11 de E8). Este documento recoge qué fixtures existen, qué prueba cada uno y el resultado de la comprobación T5 de ADR-0020 sobre un binario construido desde el commit base. No cambia formato y no escribe el registro `MIGRATION`, que sigue reservado sin escritor (`KIND_MIGRATION_RESERVED = 5`).

## 1. Fixtures de v1

| Fixture | Qué contiene | Quién lo comprueba |
|---|---|---|
| `crates/kerno-storage/tests/vectors/v1/dirs` | 33 directorios de base de datos (limpios, sucios, dañados, de versión futura) con su `EXPECTED` | `crates/kerno-storage/tests/dir_vectors.rs` (abre cada uno y compara con `EXPECTED`) |
| `crates/kerno-storage/tests/vectors/v1/structs` | Estructuras del formato de storage, positivas y negativas | `crates/kerno-storage/tests/struct_vectors.rs` |
| `crates/kerno-units/tests/vectors/v1/structs` | 33 registros de la capa de unidades v1 y 100 negativos `neg_unitNN_*` | `crates/kerno-units/tests/struct_vectors.rs` (contra un decodificador independiente) y `layer_fixtures.rs` (contra el códec de producción) |

`layer_fixtures.rs::v1_struct_vectors_are_read_by_the_production_decoder` lee cada fichero de units v1 con `Record::decode`: un positivo decodifica y un `neg_unitNN_*` se rechaza con la regla `UNIT-NN` del nombre. Fija además los recuentos (33 y 100), de modo que un vector perdido o añadido sin revisar rompe la prueba.

Resultado de las pruebas de storage en esta compilación (modo release): `dir_vectors` 8 pasan, 1 ignorada (regeneración); `struct_vectors` de storage 6 pasan, 1 ignorada; `struct_vectors` de units 9 pasan, 1 ignorada.

## 2. Fixture de ADR-0020: reconstruido

ADR-0020 (T1, T5 y T6) cita `crates/kerno-units/tests/vectors/derived-v2/` y `derived-v2/db-with-vector`, que no existían. El ADR está aceptado y no se edita. El fixture se ha reconstruido:

- `crates/kerno-units/tests/vectors/derived-v2/db-with-vector/`: una base de `kerno_db` cerrada con una unidad (`doc`, carga `hello`) y su vector (método `embed`, versión 1, perfil `model-a`). Contiene `000003.sst` y `MANIFEST`; marca de capa 2.
- `crates/kerno-units/tests/vectors/derived-v2/MANIFEST`: SHA-256, tamaño y nombre de cada fichero.

Los vectores de registro de §B de ADR-0020 (T1) ya están comprometidos de otra forma, en `derived_vectors.rs` (tabla de vectores del documento `units-v2.md`); este paquete no los duplica.

**Los bytes no se reproducen entre compilaciones.** El motor sortea la identidad de la base al crearla, y dos construcciones difieren en los bytes que la guardan. Por eso `derived_v2_directory_has_the_shape_the_build_writes` compara nombres y tamaños, y la integridad del fixture comprometido la da `MANIFEST` (`derived_v2_manifest_lists_the_committed_files`). Para regenerarlo tras un cambio de diseño deliberado:

```text
cargo test --release -p kerno-units --test layer_fixtures -- --ignored regenerate_derived_v2
```

### Pruebas de `layer_fixtures.rs`

Cada prueba de base de datos corre en `emulated_log` y en `kerno_db`. El directorio solo lo puede abrir el motor que tiene ficheros; `emulated_log` construye el mismo contenido en memoria y recibe las mismas comprobaciones.

| Prueba | Criterio de ADR-0020 |
|---|---|
| `layer1_reader_refuses_the_fixture` | T6: `LayerMeta::decode` (máximo 1) da `UNIT-16`, `found_layer_version = 2`, clase `IncompatibleLayer`; este código lee la marca con `decode_upto` |
| `this_build_opens_and_serves_the_fixture` | T7: abre, sirve 1 derivado, sin degradados ni hallazgos, `verify_units` limpio |
| `opening_the_fixture_adds_nothing` | T7: abrir la capa derivada no escribe más que la apertura del motor solo |
| `derived_v2_manifest_lists_the_committed_files` | integridad del fixture |
| `derived_v2_directory_has_the_shape_the_build_writes` | el fixture es lo que escribe la compilación (nombres y tamaños) |

## 3. Comprobación T5 sobre un binario del commit base

T5 pide que una compilación sin la característica rechace al abrir una base con derivados. El binario de referencia es `d01f646` (último commit antes de E8-04).

Procedimiento, fuera del árbol: `git archive d01f646` extraído a un directorio temporal; en él, una prueba de integración de usar y tirar abre una copia del directorio con `KernoDb::open` (sistema de ficheros real) y después `UnitDb::from_engine`; compilada en modo release con su propio directorio de destino. La copia y el destino se borraron después. La prueba de usar y tirar no se compromete: está fuera de las superficies del paquete.

Resultado:

```text
RESULT IncompatibleLayer { found: 2, supported: 1 }
BEFORE {"000003.sst": "1309 bytes crc f49aa96f", "MANIFEST": "254 bytes crc 983b12f4"}
AFTER  {"000003.sst": "1309 bytes crc f49aa96f", "LOCK": "0 bytes crc 00000000", "MANIFEST": "254 bytes crc 59f502fd"}
```

- **Cumplido:** el binario base devuelve `IncompatibleLayer` con `found = 2` y `supported = 1` (la variante que se construye desde `UNIT-16`).
- **No cumplido tal como está escrito en T5:** el «árbol de archivos idéntico antes y después» no se cumple al pie de la letra. La apertura del *motor* (que ocurre antes de que la capa lea la marca) crea `LOCK` y añade un registro de sesión a `MANIFEST`; `000003.sst` queda idéntico. Es el comportamiento del motor en cualquier apertura, también en la de una base de capa 1, y no lo causa la capa. La prueba `opening_the_fixture_adds_nothing` fija la parte que sí depende de este código: la capa no añade escrituras sobre las de la apertura del motor.
- La comprobación es una ejecución única de esta entrega, no una prueba repetible del repositorio: repetirla exige reconstruir el commit base con el procedimiento de arriba.

## 4. Estado

F-11 de E8 queda comprobada con la salvedad anterior. R-E9-13 no se produce: el fixture se pudo reconstruir. La errata de la cita de ADR-0020 (`derived-v2/db-with-vector` ya existe, pero con los ficheros de arriba) pasa al cierre E9-CL.

## 5. Fixture de E9-02: directorio con tokens

Paquete E9-02 (corregido el 2026-11-13 tras una revisión independiente, por delegación expresa del propietario). La capa 3 ([units-v3.md](../file-format/units-v3.md)) añade registros `0x70`/`0x71` y eleva la marca a 3; el precedente de §2 (un directorio comprometido, con `MANIFEST`) se repite:

- `crates/kerno-units/tests/vectors/public-ids-v3/db-with-token/`: una base de `kerno_db` cerrada con tres unidades: la 1 con token, la 2 sin él, la 3 con token y borrada (su mapeo ya no existe). Contiene `000003.sst` y `MANIFEST` del motor; marca de capa 3.
- `crates/kerno-units/tests/vectors/public-ids-v3/MANIFEST`: SHA-256, tamaño y nombre de cada fichero.
- `crates/kerno-units/tests/vectors/public-ids-v3/TOKEN`: el texto del token de la unidad 1 (sale del sistema operativo, así que cada regeneración da otro).

Como en §2, los bytes no se reproducen entre compilaciones; la integridad la da `MANIFEST`. Para regenerarlo tras un cambio de diseño deliberado:

```text
cargo test --release -p kerno-units --test public_ids -- --ignored regenerate_fixture
```

| Prueba (`public_ids.rs`) | Qué comprueba |
|---|---|
| `fixture_manifest_lists_the_committed_files` | integridad del directorio comprometido |
| `fixture_directory_opens_resolves_and_verifies` | abre, el token de `TOKEN` resuelve a la unidad 1, la 2 no tiene token, el mapeo de la 3 no existe, marca 3, `verify_units` limpio, y una unidad nueva encima recibe su propio token. En `emulated_log` construye el mismo contenido en memoria |
| `fixture_directory_is_refused_by_a_layer2_reader` | un lector de capa 2 rechaza la marca con UNIT-16 y `found_layer_version = 3` |
| `legacy_directory_written_before_tokens_opens_and_verifies` | el directorio de §2 (sin tokens, marca 2) sigue abriéndose y pasa a marca 3 con el primer token. Solo `kerno_db`: `emulated_log` no abre directorios de ficheros |

La tabla de vectores hexadecimales de los registros está en [units-v3.md](../file-format/units-v3.md) §6 y la fija `vectors_of_the_document_match_the_codec_byte_for_byte`.

## 6. Fixture de P1: directorio con relaciones de antes de la capa 4

Paquete P1 posterior a la Etapa 9 (defecto K1). La capa 4 ([units-v4.md](../file-format/units-v4.md)) añade el registro `0x72` y eleva la marca a 4 con el primer conteo. Un directorio escrito **antes** debe seguir abriéndose y funcionando; el precedente de §5 se repite:

- `crates/kerno-units/tests/vectors/rel-counts-v4/db-legacy-relations/`: una base de `kerno_db` cerrada con tres unidades y tres relaciones tal como las dejaba el escritor anterior (`REL` y sus dos entradas de adyacencia, **sin** registro de conteo), marca 1. Contiene `000003.sst` y `MANIFEST` del motor.
- `crates/kerno-units/tests/vectors/rel-counts-v4/MANIFEST`: SHA-256, tamaño y nombre de cada fichero.

**Cómo se generó, con la salvedad.** No se construyó con una compilación anterior al cambio: se escribieron las tres unidades con la capa y, por cada relación, los tres registros del escritor antiguo con `raw_commit` (el medio de daño que ya usan las pruebas del crate). Los bytes son los que el escritor anterior dejaba para una relación, pero la ruta que los escribió es la de prueba, no la de producción antigua. Como en §2 y §5, los bytes no se reproducen entre compilaciones; la integridad la da `MANIFEST`. Para regenerarlo tras un cambio de diseño deliberado:

```text
cargo test --release -p kerno-units --all-features --test rel_counts -- --ignored regenerate_fixture
```

| Prueba (`rel_counts.rs`) | Qué comprueba |
|---|---|
| `fixture_manifest_lists_the_committed_files` | integridad del directorio comprometido |
| `legacy_directory_opens_works_and_upgrades_when_written` | abre con marca 1, sin conteos, `verify_units` limpio, sirve sus relaciones, abrir y leer no escriben; una relación nueva sube la marca a 4 y escribe los dos conteos de sus extremos. En `emulated_log` construye el mismo contenido en memoria |
| `legacy_relations_work_and_get_a_count_when_next_changed`, `deleting_a_legacy_relation_counts_what_is_left` | el mismo camino, en los dos motores, con el límite de units-v4 §7 punto 1 fijado |
| `layer3_reader_refuses_a_database_with_counts` | un lector de capa 3 rechaza la marca con UNIT-16 y `found_layer_version = 4` |
| `vectors_of_the_document_match_the_codec_byte_for_byte` | la tabla de units-v4 §6 |
