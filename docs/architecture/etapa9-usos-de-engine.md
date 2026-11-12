# Accesores de `UnitDb` que sustituyen a `engine()` (E9-08a, corregido en E9-08f y E9-08g)

| Campo | Valor |
|---|---|
| Paquete | E9-08a de [la acta de la Etapa 9](etapa9-acta.md) §3; correcciones E9-08f (decisión D23 de §8.1) y E9-08g |
| Regla | §4, regla de la envoltura: ningún accesor devuelve el motor ni una referencia a él |
| Alcance | `crates/kerno-units/src/db.rs` y `crates/kerno-units/Cargo.toml` (característica `test-support`) |
| Estado del código | `UnitDb::engine()` ya no existe: `grep -rn "pub fn engine" crates/kerno-units/src` no devuelve nada. El motor es un campo `pub(crate)`; fuera de `db.rs` solo lo usan `compact` (retención) y `reclaim` (E9-07), a través de `inner_engine()` y del rasgo público `Reclaim` de `kerno-core` |

## 1. Correspondencia con los usos antiguos

Los usos de `engine()` se repartían por el método que se invocaba sobre el motor. Como la función ya no existe, ninguna búsqueda reproduce los recuentos antiguos, y este documento no da ninguno. Cada uso antiguo tiene hoy su accesor:

| Uso antiguo | Accesor actual |
|---|---|
| `.snapshot()` (claves crudas, volcados, `seq`, vistas retenidas) | `raw_snapshot()`, y `raw_get(key)` / `raw_seq()` cuando solo se lee un valor o la secuencia |
| `.commit(batch)` (dañar o reparar bytes) | `raw_commit(entries)` |
| `.stats()` | `storage_stats()` |
| `.flush()` | `hook_flush()` |
| `.wait_idle()` | `hook_wait_idle()` |
| `.compact_range()` | `hook_compact_range(start, end)` |

Los auxiliares de las pruebas (`settle`, `stat`, `put_all`, `undo_of`…) están escritos sobre estos accesores.

## 2. Accesores

Todos en `UnitDb<E>`.

| Accesor | Requiere | `#[doc(hidden)]` | Necesita `test-support` | Devuelve | Nota |
|---|---|---|---|---|---|
| `raw_snapshot()` | `E: TxnEngine` | Sí | No | `E::Snapshot` | Vista de solo lectura; no es el motor. Llama a la misma comprobación de envenenamiento que `snapshot` |
| `raw_get(key)` | `E: TxnEngine` | Sí | No | `Option<Vec<u8>>` | Lee a través de `raw_snapshot`: misma comprobación |
| `raw_seq()` | `E: TxnEngine` | Sí | No | `u64` | Lee a través de `raw_snapshot`: misma comprobación |
| `storage_stats()` | `E: TxnEngine` | No | No | `Vec<(String, u64)>` | Los contadores del motor por nombre. Vacío en `emulated_log` (`Emulated<LogEngine>` no tiene contadores); no vacío en `kerno_db` |
| `raw_commit(entries)` | `E: TxnEngine` | Sí | **Sí** | `u64` | Un commit del motor por debajo de la capa; `None` borra la clave y, si una clave se repite, gana la última entrada. Solo existe para pruebas que dañan bytes almacenados. No pasa por la época de política (ni la caché de política ni el cursor de retención ven el cambio), ni por la comprobación de envenenamiento, ni retira los registros derivados ni la supresión que retira un borrado de la capa |
| `hook_flush()`, `hook_wait_idle()`, `hook_compact_range(start, end)` | `E: TxnEngine + TestHooks` | Sí | **Sí** | `()` | Ganchos de prueba del motor; hoy solo `KernoDb` los implementa |

`test-support` está declarada en `crates/kerno-units/Cargo.toml` y apagada por defecto. Sin ella, una compilación no tiene camino de escritura por debajo de la capa ni ganchos de compactación de rangos crudos arbitrarios. Las llamadas públicas de reclamación son dos, ambas sobre el rasgo público `Reclaim` de `kerno-core` y no sobre `TestHooks`: `reclaim()` (E9-07, D14), que compacta todo el espacio de claves, y `compact()` (retención, E3-06), que compacta los rangos de revisiones (`REV_META` y `REV_PAYLOAD`). Las pruebas unitarias del propio crate ven `raw_commit` y los ganchos con `cfg(test)`. Las pruebas de integración y los ejemplos que los llaman declaran `required-features = ["test-support"]`: `cargo test -p kerno-units` sin la característica compila y se salta esos objetivos, y `cargo test -p kerno-units --features test-support` (o `--all-features`) los ejecuta. Los ejemplos se lanzan con `cargo run --release -p kerno-units --features test-support --example <nombre>`.

Ningún accesor devuelve `&E`, `E` ni el `UnitDb`.

## 3. Uso actual

Recuento de llamadas, no de líneas, con `grep -rnoE "\.(raw_snapshot|raw_get|raw_seq|raw_commit|storage_stats|hook_flush|hook_wait_idle|hook_compact_range)\(" crates examples --include=*.rs`: cada coincidencia es una llamada, y una línea con dos llamadas cuenta dos. Incluye las llamadas internas de `db.rs` (`raw_get` y `raw_seq` llaman a `raw_snapshot`) y las pruebas de `accessor_tests`.

| Accesor | Llamadas |
|---|---|
| `raw_snapshot` | 50 |
| `raw_commit` | 26 |
| `raw_get` | 20 |
| `hook_wait_idle` | 13 |
| `storage_stats` | 9 |
| `hook_flush` | 8 |
| `raw_seq` | 7 |
| `hook_compact_range` | 6 |
| **Total** | **139** (en 137 líneas) |

## 4. Verificación

Pruebas en `crates/kerno-units/src/db.rs` (`accessor_tests`), sobre `Emulated<LogEngine>` (`emulated_log`) y `KernoDb` (`kerno_db`):

- `raw_commit` con varias entradas en un solo commit y con claves repetidas (gana la última, sea escritura o borrado): la secuencia avanza exactamente lo que avanza un commit de una entrada, de modo que un commit por entrada no pasa; `raw_get` con dos claves distintas guardadas y una ausente; una vista de `raw_snapshot` tomada antes de un commit conserva su valor.
- `raw_snapshot`, `raw_get` y `raw_seq` devuelven `Storage(Poisoned)` en una base envenenada, igual que `snapshot`.
- `storage_stats` vacío en `emulated_log`; en `kerno_db`, no vacío y con el contador `commits` aumentado en dos tras dos commits. Un contador que el motor no informa hace fallar la prueba; no se lee como cero.
- Ganchos, solo en `kerno_db`:
  - `hook_flush` sube `flushes` en uno y vacía la memtable.
  - `hook_wait_idle` espera el volcado en segundo plano de una memtable rotada (con latencia en cada sincronización del disco simulado) hasta `immutable_memtables = 0`. El bucle que provoca la rotación tiene un máximo de 64 lotes y falla si `memtable_rotations` no cambia en ellos. Que la prueba distinga un `hook_wait_idle` que no espera depende de que el volcado, frenado por la latencia, siga en curso al leer los contadores; no está garantizado por construcción.
  - `hook_compact_range` con límites acotados y no simétricos (`Included(clave 10)` / `Excluded(clave 20)`), con las lápidas de las claves 10 a 19 y las de las claves 20 a 31 en dos tablas de L0 separadas: se descartan exactamente las 10 lápidas del rango, y una compactación posterior de todo el espacio descarta las 12 de fuera, que seguían ahí. Con límites no acotados (22 descartadas), invertidos (ninguna) o con el extremo final incluido (22) la prueba falla.
