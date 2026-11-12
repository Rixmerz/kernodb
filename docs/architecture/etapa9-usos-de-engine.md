# Usos de `UnitDb::engine()` por método y accesores (E9-08a)

| Campo | Valor |
|---|---|
| Paquete | E9-08a de [la acta de la Etapa 9](etapa9-acta.md) §3 |
| Regla | §4, regla de la envoltura: ningún accesor devuelve el motor ni una referencia a él |
| Alcance | Solo añade accesores en `crates/kerno-units/src/db.rs`. `engine()` no cambia aquí; lo retira E9-08e |

## 1. Clasificación

Recuento con `grep -rn "engine()" crates examples --include=*.rs`: 135 líneas, de ellas 2 no son llamadas (nombres de pruebas en `kerno-testkit`). Las 133 llamadas restantes se reparten por el método que se invoca sobre el motor (o por la función a la que se pasa):

| Uso | Llamadas | Ejemplo | Accesor |
|---|---|---|---|
| `.snapshot()` (lectura de claves crudas, volcados, `seq`, vistas retenidas) | 46 | `db.engine().snapshot().unwrap()` | `raw_snapshot()`, y `raw_get(key)` / `raw_seq()` cuando solo se lee un valor o la secuencia |
| `.commit(batch)` / `.begin_write()` + `commit` (dañar o reparar bytes) | 7 | `put_all(db.engine(), patch)` | `raw_commit(entries)` |
| `.stats()` | 2 | `stored_bytes(&self.engine().stats())` | `engine_stats()` |
| `.flush()` | 2 | `db.engine().flush()` | `engine_flush()` |
| `.wait_idle()` | 4 | `db.engine().wait_idle()` | `engine_wait_idle()` |
| `.compact_range()` | 1 | `db.engine().compact_range(..)` | `engine_compact_range(start, end)` |
| Pasado a una función auxiliar (`settle`, `stat`, `put_all`, `undo_of`, `damage_head`, `apply`, `TestHooks::flush`…) | 71 | `put_all(db.engine(), patch)` | el auxiliar se reescribe sobre el accesor de su fila (`raw_commit`, `raw_snapshot`, `engine_stats`, los tres de `engine_*`) |

Los totales por fila son los de la búsqueda; la última fila agrupa los usos en que la llamada al motor ocurre dentro del auxiliar. El reparto por paquete de migración es el de §3 de la acta (E9-08b a E9-08d).

## 2. Accesores añadidos

Todos en `UnitDb<E>`; los de escritura y de ganchos son `#[doc(hidden)]`.

| Accesor | Requiere | Devuelve | Nota |
|---|---|---|---|
| `raw_snapshot()` | `E: TxnEngine` | `E::Snapshot` | Vista de solo lectura; no es el motor |
| `raw_get(key)` | `E: TxnEngine` | `Option<Vec<u8>>` | Datos |
| `raw_seq()` | `E: TxnEngine` | `u64` | Datos |
| `raw_commit(entries)` | `E: TxnEngine` | `u64` | Escribe por debajo de la capa a propósito (pruebas que dañan bytes); `None` borra la clave |
| `engine_stats()` | `E: TxnEngine` | `Vec<(String, u64)>` | Datos |
| `engine_flush()`, `engine_wait_idle()`, `engine_compact_range(start, end)` | `E: TxnEngine + TestHooks` | `()` | Hoy solo `KernoDb` implementa los ganchos. La reclamación pública es de E9-07; este acceso es de pruebas |

Ningún accesor devuelve `&E`, `E` ni el `UnitDb`. `raw_snapshot` devuelve un `E::Snapshot`, una vista de lectura que ya devolvía `Engine::snapshot`; no permite escribir.

## 3. Verificación

Pruebas en `crates/kerno-units/src/db.rs` (`accessor_tests`), sobre `Emulated<LogEngine>` (`emulated_log`) y `KernoDb` (`kerno_db`); los ganchos solo existen en `KernoDb`. Cada prueba se vio fallar con una mutación sobre una copia del árbol: borrar como poner un valor vacío, `raw_seq` fijo en 0, `raw_get` siempre `None`, `engine_stats` vacío.
