# ADR-0020 — Persistencia de vectores como derivados

| Campo | Valor |
|---|---|
| Estado | **Aceptado el 2026-11-12, por delegación del propietario** en las decisiones pendientes de la hoja de ruta ese día (acta de la Etapa 8, D2, con la condición de E8-03). **Revocable por el propietario**: mientras no lo revoque, rige. La condición de D2 se cumple: E8-03 pasó (ver [Evidencia](#evidencia)) |
| Fecha | 2026-11-12 |
| Decisores | Propietario del proyecto, por delegación (2026-11-12) |
| Relacionado con | [ADR-0018](0018-versionado-y-reconstruccion-de-derivados.md) (registro común de derivados, opción 2); [ADR-0011](0011-vectores-y-recuperacion-hibrida.md) (en su cláusula de persistencia, reemplazada por este ADR; el ADR-0011 no se edita); [ADR-0003](0003-formato-fisico-y-versionado.md) §1, §3, §4; [ADR-0007](0007-consistencia-de-indices.md); [ADR-0008](0008-serializacion-documental-y-hashes.md) (BLAKE3-256); [ADR-0009](0009-supresion-retencion-y-backups.md) (opción 3); [ADR-0013](0013-formato-v1-y-protocolo-del-motor.md) (formato v1 congelado, no editado); [ADR-0014](0014-representacion-fisica-de-relaciones.md); [formato de la capa de unidades v1](../file-format/units-v1.md) (no editado); [acta de la Etapa 8](../architecture/etapa8-acta.md) §5.4, §7 y §8.1 (D2); invariantes I-1, I-2, I-21, I-23, I-24, I-25, I-31, I-37, I-38, I-42 (propuesta) |

## Contexto

Hoy no se persiste ningún vector. Los vectores los entrega quien llama y viven en memoria (`VectorSet`, ADR-0011). El registro de derivados de E8-02 y la obsolescencia de E8-03 también viven solo en memoria: al reabrir, el llamador vuelve a dar la lista de descriptores (`DerivedDb::open`). El acta de la Etapa 8 (D2, §8.1) eligió persistir los vectores como derivados, con un bit de característica `incompat` y un ADR nuevo, condicionado a que E8-03 pase. ADR-0011 pide además que persistir vectores tenga un ADR que lo reemplace.

Persistir cambia el formato. El formato v1 del motor está congelado (ADR-0013) y no puede cambiar: ni sus bytes, ni su lista de bits conocidos, ni sus vectores. Este ADR fija dónde viven los vectores y sus descriptores, cómo se declara la compatibilidad, cómo se atomizan la escritura y la obsolescencia, qué pasa al reabrir, qué sobrevive a una supresión, qué queda fuera y qué debe cumplir el paquete E8-04.

Este ADR fija los bytes de la capa, no los escribe. El código es de E8-04.

## Condición de D2: E8-03 pasó

La condición de D2 («persistir solo si E8-03 pasa») se cumple. Se ejecutó en este árbol (commit `d01f646`, fusión de E8-03):

```
cargo test -p kerno-units --test derived_registry --test derived_stale
  Running tests/derived_registry.rs
  test result: ok. 13 passed; 0 failed; 0 ignored
  Running tests/derived_stale.rs
  test result: ok. 46 passed; 0 failed; 0 ignored
```

## Opciones consideradas

Cada punto indica la opción elegida (**en negrita**) y las descartadas.

### 1. Dónde viven un vector y su descriptor

1. **Espacio de claves de la capa de unidades, con tags `0x60`, `0x61` y `0x62`** (nibble alto `0x6`, hoy sin asignar).
   - **Las claves van por el mismo motor transaccional que las unidades, con su propio prefijo de tag, y la capa las lee y escribe dentro de sus `ChangeSet`.** Así la atomicidad con la fuente (I-21) es la del motor, sin protocolo nuevo.
   - En la capa 1, el nibble `0x6` cae dentro de «una capa más nueva» de UNIT-2 (`units-v1.md` §5): una capa 1 que se encuentre estas claves las rechaza, en vez de leerlas mal.
2. Claves directamente en el espacio del motor, sin capa. Rechazada: el motor no conoce los derivados ni la marca de capa, y la capa perdería el control de qué es un descriptor válido.
3. Vectores en un archivo lateral con su propio formato. Rechazada: rompe I-21 (el vector y su fuente no se confirman en el mismo commit), y obliga a un segundo protocolo de durabilidad y recuperación.
4. Tags en `0x43`–`0x4F`. Rechazada: el nibble `0x4` ya es de relaciones (`0x40`–`0x42`), y dejarlo mezclado con ellas complica la lectura de los rangos de adyacencia.

### 2. El bit de característica: nombre, capa y momento

1. **Versión 2 de la capa en la marca `LAYER_META "layer"`, con la característica `DERIVED_VECTORS`.** La capa 1 rechaza en la apertura cualquier marca con `layer_version > 1` (UNIT-16, `IncompatibleLayer`). Es la forma de «incompat» que ya existe en el código: la apertura decodifica la marca antes que ninguna otra clave (`db.rs`, `read_mark`).
2. Bit en la palabra `incompat` de la cabecera común del motor (`ADR-0003` §1, `v1.md` §4). Rechazada por cuatro motivos:
   - Los archivos ya escritos no llevan el bit, y ponérselo a todos es reescribir el motor. Solo el MANIFEST se reescribe en cada *checkpoint*, así que el bit tendría que vivir en él y en cada archivo que una build vieja pudiera leer.
   - El motor tendría que saber qué es un derivado. Hoy no lo sabe, y no tiene ningún concepto de capa.
   - Cambia `KNOWN_INCOMPAT` (`crates/kerno-storage/src/format/features.rs`), la lista que aplica HDR-6 a todo archivo. Eso es un cambio de reglas del formato congelado, con edición de `v1.md` §4 y regeneración de vectores (`v1.md` §15).
   - La build vieja solo falla al abrir si el bit está en el MANIFEST, que se reemplaza en cada *checkpoint*. Hay que escribirlo atómicamente con la primera escritura de un vector, lo que acopla el motor a la capa.
3. Un registro con nombre dentro de `LAYER_META`, con una palabra de características (como `policy/<tipo>`). Rechazada: la apertura solo lee la marca `layer` (`read_mark`). Una build vieja ignoraría el registro nuevo y abriría la base, que es exactamente lo que el bit debe impedir.
4. Una palabra de características dentro de la marca, cambiando su valor de 7 bytes. Rechazada: cambia los bytes de un valor de la capa 1. UNIT-10 rechaza bytes sobrantes, así que una build vieja lo clasificaría como `Corruption`. Hay que decir que es incompatibilidad, no daño (criterio del ADR-0013, punto 8).
5. Marca siempre en 2 para toda base nueva. Rechazada: dejaría de abrir en las builds de capa 1 bases que no tienen ningún vector. El acta pide que el bit se active solo con un vector persistido.

El bit se activa **solo cuando una base tiene un vector persistido**, y **nunca se baja**. Una base que tuvo un vector y luego perdió todos queda en versión 2: bajarla exigiría saber que ningún archivo vivo ni ningún snapshot retiene una clave `0x6x`, y el motor no garantiza eso sin compactar (ADR-0009, opción 3).

### 3. Atomicidad

1. **El vector, su descriptor, su entrada de índice inverso y, si hace falta, la marca de capa, se escriben en la misma transacción del motor (`begin_write` … `commit`).** La marcación de obsolescencia por un cambio de fuente va en la transacción de ese cambio.
2. Marcar después del commit, en memoria, como hace E8-03 (`DerivedChangeSet::commit`). Rechazada para datos persistidos: entre el commit y la marca, una caída deja en disco un descriptor `ready` con una fuente que ya cambió. Es aceptable mientras el registro sea de memoria, y no vale para persistido.

### 4. Reconstrucción y pérdida

1. **La base manda; un vector persistido es una copia verificada de un cálculo, que se puede tirar y rehacer.** Al abrir se reconstruye el registro desde los descriptores persistidos, se comprueba cada vector contra su hash de salida, y lo que no cuadra no se sirve.
2. Reconstruir todo al abrir con la función de cálculo del llamador. Rechazada: el cálculo es caro y lo aporta quien llama; abrir no debe calcular nada.
3. Fiarse del índice inverso sin comprobarlo contra los descriptores. Rechazada: un índice inverso equivocado haría una obsolescencia que no ocurre, o al revés.
4. Reparar al abrir, escribiendo. Rechazada: `UnitDb::open` no escribe nada (`db.rs`, documentación de `open`). La reparación es una llamada explícita que escribe un `ChangeSet` de propósito `Derived`.

### 5. Supresión

1. **Borrado en el mismo `ChangeSet` de la fuente, y el motor retira los archivos al compactar** (ADR-0009, opción 3, con la clase de vectores asignada a la opción 1 y el borrado de la fuente como supresión). Es lo que pide la sección 5.3 del acta.
2. Marcar `stale` y conservar el descriptor, como hace E8-03 al borrar una unidad. Rechazada: un derivado de una fuente borrada no se puede reconstruir, y ADR-0009 pide que la supresión llegue a los derivados, no que queden como obsoletos.
3. Borrado criptográfico. No disponible: depende de ADR-0006, pendiente (ADR-0009, punto 4).
4. Prometer borrado físico. Rechazada: ADR-0009, punto 3, no lo promete.

### 6. Fuera de alcance

- Índice aproximado en disco.
- Modelo en el núcleo, y red (I-31).
- Cifrado (ADR-0006).
- Resúmenes con varias fuentes (E8-06) y relaciones inferidas (acta D8).
- Borrado físico.
- Política de dimensión por perfil más allá de lo que ya hace `VectorSet` (una dimensión por conjunto, `DimensionMismatch`).

## Criterios de decisión

Fijados antes de implementar E8-04:

- Ningún byte de formato v1 del motor cambia: `KNOWN_INCOMPAT`, `WRITE_INCOMPAT`, `KNOWN_RO_COMPAT`, `v1.md` y los vectores de `kerno-storage` quedan como están.
- Una build que no conoce la característica **rechaza**, nunca lee mal. El rechazo ocurre antes de leer ninguna otra clave, y no escribe nada.
- Un vector nunca es fuente de verdad: lo que falte se rehace desde las unidades (I-24), y no se sirve nada que no tenga su descriptor verificado.
- El vector, su descriptor y su índice inverso se confirman juntos o no se confirman (I-21).
- La obsolescencia de un vector ocurre en la misma transacción que el cambio de su fuente (I-23, I-25, I-38).
- Un borrado de fuente no deja ningún vector visible después del commit (I-1, I-42).

## Decisión

**Los vectores se persisten como derivados de la capa de unidades, con las reglas siguientes.**

1. **Espacio de claves:** tags `0x60` (descriptor), `0x61` (vector) y `0x62` (índice inverso fuente → derivado), en la capa de unidades, nunca en el formato del motor.
2. **Bit:** característica `DERIVED_VECTORS`, declarada como **versión 2 de la capa** en la marca `LAYER_META "layer"`. Es `incompat` en el sentido del ADR-0003: una build de capa 1 rechaza al abrir. La versión 2 se escribe en el primer `ChangeSet` que persiste un derivado, nunca se baja, y no se escribe en bases sin vectores.
3. **Atomicidad:** descriptor, vector, entrada de índice inverso y marca de capa se confirman en una transacción. Un cambio de fuente marca `stale` a sus derivados en la misma transacción, y un borrado de fuente los retira en la misma transacción.
4. **Reconstrucción:** la base es la fuente de verdad. Al abrir, el registro se reconstruye desde los descriptores persistidos; los vectores se comprueban y no se sirven si no cuadran.
5. **Supresión:** un borrado de unidad o una retención de revisión retiran en la misma transacción los descriptores, los vectores y las entradas de índice inverso de sus derivados. Lo que sobrevive en disco hasta la compactación se declara como límite (ADR-0009, opción 3), sin prometer borrado físico.
6. **Este ADR reemplaza, en lo que dice de persistencia, la cláusula de ADR-0011** («Comprometido» y «Revisar» de sus Consecuencias). Ese ADR no se edita, y su estado no cambia: el resto de ADR-0011 (búsqueda exacta, perfil de modelo, dimensión única por conjunto) sigue vigente.

Las reglas de bytes, la atomicidad, la reconstrucción y la supresión están especificadas en las secciones siguientes. Los criterios de aceptación de E8-04 están en la sección [Criterios de aceptación de E8-04](#criterios-de-aceptación-de-e8-04).

## Formato de bytes (espacio de claves y valores)

Convenciones de `units-v1.md` §1, que se mantienen: enteros **big-endian**, `str(max)` = `len: u32` (big-endian) ‖ bytes UTF-8, `bytes(max)` con `len` antes, `unit_id` de 16 bytes, `ContentHash` no aparece aquí. Hay **una excepción**, explícita porque es un hecho del código de E8-02/E8-03: las **preimágenes de los hashes** usan enteros **little-endian** (ver §C).

### A. Claves (capa 2)

| `tag` | Nombre | Campos tras el tag | Longitud |
|---|---|---|---|
| `0x60` | `DERIVED_DESC` | `derived_id[32]` | 33 |
| `0x61` | `DERIVED_VEC` | `derived_id[32]` | 33 |
| `0x62` | `DERIVED_SRC` | `unit_id[16]` ‖ `rev_no: u64` ‖ `derived_id[32]` | 57 |

- `derived_id` es el identificador de la sección C. Cada derivado es un descriptor, como mucho un vector y exactamente una entrada `DERIVED_SRC`.
- `DERIVED_SRC` ordena por unidad, luego por revisión: «todos los derivados de una unidad» es un rango con prefijo `0x62 ‖ unit_id`, y «todos los de una revisión» es un rango con prefijo `0x62 ‖ unit_id ‖ rev_no`.
- Tags `0x63`–`0x6F`: reservados. Una capa 2 que los encuentre los rechaza con UNIT-2 (`IncompatibleLayer`).

### B. Valores

**B.1 `DERIVED_DESC` (`0x60`).** Campos en este orden:

| # | Campo | Tipo | Restricción |
|---|---|---|---|
| 1 | `envelope_version` | `u8` | `1` (UNIT-6, UNIT-7) |
| 2 | `reserved_flags` | `u8` | `0` (UNIT-8) |
| 3 | `kind` | `u8` | `1` = `Vector`. `0` y `2..=255` son corrupción (DV-1) |
| 4 | `state` | `u8` | `1` = `Ready`, `2` = `Stale`, `3` = `Superseded`. Los demás son corrupción (DV-2) |
| 5 | `source_unit` | `[u8; 16]` | ninguna |
| 6 | `source_rev` | `u64` | ≥ 1 (DV-4) |
| 7 | `method` | `str(64)` | 1..=64 B, UTF-8 (DV-4) |
| 8 | `method_version` | `u32` | ninguna |
| 9 | `profile` | `str(64)` | 1..=64 B, UTF-8 (DV-4). Es el perfil de modelo de ADR-0018 y de I-25; el núcleo lo guarda como texto opaco y no lo interpreta |
| 10 | `param_count` | `u8` | 0..=16 (DV-5) |
| 11 | `params` | `param_count` × (`key: str(64)` ‖ `value: str(256)`) | `key` 1..=64 B, `value` 0..=256 B; las claves en **orden creciente de bytes**, sin repetidas (DV-5) |
| 12 | `input_hash` | `[u8; 32]` | §C.2 |
| 13 | `output_hash` | `[u8; 32]` | §C.3 |

Sin bytes sobrantes (UNIT-10). **El descriptor no lleva fecha**: ADR-0018 la pide, pero una fecha de pared haría que la reconstrucción no fuera igual a la construcción (I-24). La fecha de un derivado es el `created_at` del `CHANGESET` de su commit.

**B.2 `DERIVED_VEC` (`0x61`).**

| # | Campo | Tipo | Restricción |
|---|---|---|---|
| 1 | `envelope_version` | `u8` | `1` |
| 2 | `reserved_flags` | `u8` | `0` |
| 3 | `dim` | `u32` | 1 ≤ `dim` ≤ 4096 (DV-6) |
| 4 | `elem_type` | `u8` | `1` = `f32`, IEEE 754 binary32. Otro valor es corrupción en la capa 2 (DV-7); un tipo nuevo exige una versión de capa nueva |
| 5 | `data` | `dim` × 4 B | cada elemento, sus bits en **big-endian**; finito (DV-8) |

Longitud total: `7 + 4·dim` bytes (DV-6). Todo elemento es finito: `NaN`, `+∞` y `−∞` son corrupción. El `−0.0` (bits `0x80000000`) es corrupción: el escritor lo escribe como `+0.0`, para que el hash de salida no dependa del signo de cero (DV-8).

**B.3 `DERIVED_SRC` (`0x62`).** Valor de 2 bytes: `envelope_version = 1`, `reserved_flags = 0`. Sin más campos (UNIT-10). La clave lleva la fuente; el valor no repite nada.

**B.4 Marca de capa, versión 2.** Mismo valor que `units-v1.md` §3.1, con `layer_version = 2` (`u32` big-endian). Registro `LAYER_META` con nombre `layer`, sin cambios de forma. Una marca con `layer_version > 2` es `IncompatibleLayer` (UNIT-16 extendido).

**B.5 `CHANGESET`, propósito nuevo.** `purpose = 5` = `Derived`: lo escribe la capa para un `ChangeSet` que solo escribe claves `0x60`–`0x62`, sin miembros, como `Relation` (3) e `Index` (4). Un `ChangeSet` de propósito `Derived` con `member_count ≠ 0`, o que toque claves de unidades o de relaciones, es corrupción (DV-13).

### C. Identidad y hashes

Todas las funciones usan BLAKE3-256 (ADR-0008). `F(x)` es el campo de longitud `u64` **little-endian** (8 bytes) seguido de `x`.

**C.1 `derived_id`** (32 bytes; es la clave de `0x60`, `0x61` y `0x62`):

```
derived_id = BLAKE3-256(
    "kerno.derived.identity.v2"                      -- ASCII, sin F(·)
    ‖ F(u64le(n))                                    -- n = número de fuentes (1 en vectores)
    ‖ por cada fuente, en orden (unit, rev):
        F(unit[16]) ‖ F(u64le(rev))
    ‖ F(method) ‖ F(u32le(method_version))
    ‖ F(profile)
    ‖ F(u64le(m))                                    -- m = número de parámetros
    ‖ por cada parámetro, en orden creciente de clave:
        F(key) ‖ F(value)
)
```

Fuera del `id`: `input_hash`, `output_hash` y la fecha. Así, reconstruir el mismo dato sobre entradas iguales da el mismo `id`.

Cambio respecto de E8-02: el `id` v1 (`IDENTITY_DOMAIN = "kerno.derived.identity.v1"`, `derived.rs`) no tiene `profile`. E8-04 añade el campo y cambia el dominio a `…v2`. El `id` v1 nunca se persistió, así que no hay datos que migrar.

**C.2 `input_hash`** (32 bytes; el de `derived_stale.rs`, sin cambios). Aquí **no** hay `F(·)` y los enteros son little-endian:

```
input_hash = BLAKE3-256(
    "kerno.derived.input.v1"                          -- ASCII
    ‖ u64le(n)                                        -- n = número de fuentes
    ‖ por cada fuente, en orden (unit, rev):
        unit[16] ‖ u64le(rev)
)
```

**C.3 `output_hash`** (32 bytes): `BLAKE3-256` de todos los bytes del valor `DERIVED_VEC` (`7 + 4·dim` bytes, incluido el encabezado). Lo fija el escritor, y el lector lo comprueba antes de servir el vector (DV-12).

### D. Reglas de validación

Orden de evaluación, como en `units-v1.md` §6: reglas de clave, luego envelope, luego campos en orden, luego reglas entre campos. Las reglas de la capa 1 (UNIT-6, UNIT-7, UNIT-8, UNIT-9, UNIT-10, UNIT-11, UNIT-12) se aplican igual a los valores nuevos.

| Id | Regla | Clase |
|---|---|---|
| DV-1 | `kind ∉ {1}` | `Corruption` |
| DV-2 | `state ∉ {1, 2, 3}` | `Corruption` |
| DV-3 | Longitud de clave ≠ 33 (`0x60`, `0x61`) o ≠ 57 (`0x62`) | `Corruption` |
| DV-4 | `source_rev = 0`; `method` o `profile` vacío, mayor de 64 B o no UTF-8 | `Corruption` |
| DV-5 | `param_count > 16`; clave vacía o mayor de 64 B; valor mayor de 256 B; claves no crecientes o repetidas; no UTF-8 | `Corruption` |
| DV-6 | `dim = 0`, `dim > 4096`, o longitud del valor ≠ `7 + 4·dim` | `Corruption` |
| DV-7 | `elem_type ≠ 1` | `Corruption` |
| DV-8 | Elemento `NaN`, `±∞` o `−0.0` | `Corruption` |
| DV-9 | Vector o entrada `0x62` sin descriptor; descriptor sin entrada `0x62`; o entrada `0x62` cuyo `unit`/`rev` no son los del descriptor | `Corruption` |
| DV-10 | El `derived_id` recalculado (§C.1) ≠ la clave | `Corruption` |
| DV-11 | Claves `0x60`–`0x62` en una base cuya marca tiene `layer_version = 1` | `Corruption` |
| DV-12 | `Ready` sin vector, o con `output_hash` distinto del hash del valor; `Stale` o `Superseded` con vector | `Corruption` |
| DV-13 | `purpose = 5` con `member_count ≠ 0`; o un `ChangeSet` de propósito `Derived` que toca claves de unidades o relaciones | `Corruption` |
| DV-14 | Un descriptor cuya unidad fuente no existe, o cuya cabeza está en `Deleted`: la supresión de §S5 debía haberlo retirado | `Corruption` |
| UNIT-2 (extendido) | Tags de nibble `0x4`–`0xF` fuera de `0x40`–`0x42`, `0x50`–`0x52` y `0x60`–`0x62`: lo escribió una capa más nueva | `IncompatibleLayer` |
| UNIT-16 (extendido) | `layer_version > 2` | `IncompatibleLayer` |

Un descriptor que falla DV-10 no se carga: no se sirve su vector ni se ocupa su entrada en el registro. Su clave y sus hijas quedan como huérfanas (ver §S4).

## Atomicidad (S3)

Un `ChangeSet` en la capa 2 es una sola transacción del motor. Dentro de ella:

1. **Escribir un derivado** (descriptor `Ready`, vector y entrada `0x62`) solo es válido si la revisión fuente **es la cabeza de su unidad en esa misma transacción**. Si no lo es, la escritura se rechaza con `InvalidArgument` y nada se escribe. Así no se confirma un `Ready` sobre una revisión vieja.
2. **Un cambio de fuente** (crear, actualizar, archivar, desarchivar o restaurar una unidad: cualquier revisión que no sea `Delete`) recorre, con el prefijo `0x62 ‖ unit_id`, las entradas de la unidad. Para cada descriptor en `Ready`, escribe `state = Stale` y borra su vector (DV-12). En la misma transacción. Un derivado en `Stale` o `Superseded` no se toca.
3. **Un borrado de unidad** (revisión `Delete`) borra, con el mismo recorrido, descriptores, vectores y entradas de sus derivados. No los deja como obsoletos (§S5).
4. **La marca de capa** se escribe en la misma transacción que la primera clave `0x60`–`0x62`. Si la base está en versión 1, la transacción escribe la marca con `layer_version = 2`, y si ya está en 2, no la toca.
5. **Ningún escritor baja la marca.** `UnitDb::set_policy` y el primer `ChangeSet` de una base nueva escriben `LAYER_VERSION` hoy con la constante 1 (`db.rs`, líneas 625 y 1373). En la capa 2 escriben `max(marca actual, 2 si la base ya tiene derivados)`.

**Lo que cambia respecto de E8-03.** La marcación de E8-03 ocurre **después** de `self.cs.commit()?`, en memoria. Para persistidos ocurre **dentro** de la transacción. La regla de I-23 se cumple porque el estado persistido y la fuente cambian en el mismo commit, no porque un lector espere a un bloqueo en memoria.

Un fallo de commit (por ejemplo, la sincronización de datos) no deja nada escrito: ni la fuente, ni la marca, ni el derivado. Eso lo garantiza el motor (ADR-0002).

## Reconstrucción y pérdida (S4)

**Principio.** Un vector persistido nunca es fuente de verdad. La fuente es la unidad. Si un vector se pierde, se corrompe o no cuadra, se rehace con `rebuild` desde la revisión fuente, y el resultado es igual al de construir de nuevo (I-24): mismo `derived_id`, mismo `input_hash`, mismo `output_hash`, mismos bytes de vector.

**Reabrir.** `DerivedDb::open` deja de recibir la lista de descriptores cuando hay persistencia: el registro sale de la base. Sin escribir nada, en este orden:

1. Leer la marca de capa. Versión 1 con claves `0x60`–`0x62`: `Corruption` (DV-11). Versión mayor que 2: `IncompatibleLayer`.
2. Recorrer `0x60`. Decodificar cada descriptor, recalcular su `derived_id` (DV-10) y registrar en el registro los que pasan, con el estado persistido.
3. Recorrer `0x62` y cruzarlo con los descriptores (DV-9). El registro en memoria se construye a partir de los **descriptores**, no del índice inverso: el índice persistido se verifica contra ellos.
4. Para cada `Ready` que pasa, comprobar que la cabeza de su fuente tiene la revisión y el `input_hash` del descriptor. Si no cuadra, pasa a `Stale`. Si la cabeza cuadra, cargar su vector y comprobar el `output_hash` (DV-12). Si falla, pasa a `Stale` y se informa.
5. Los vectores sin descriptor `Ready` no se cargan.

**Qué pasa a cada caso** (todos se prueban en E8-04):

| Caso | Lo que sirve la lectura | Lo que informa `verify_units` | Quién lo corrige |
|---|---|---|---|
| Vector `0x61` sin descriptor `0x60` | Nada: no se carga | DV-9 | Reparación explícita (`ChangeSet` `Derived` que lo borra) |
| Descriptor sin entrada `0x62`, o entrada distinta | El registro usa el descriptor; el estado no cambia | DV-9 | Reparación explícita, que reescribe `0x62` |
| Descriptor cuyo `derived_id` no cuadra con su clave | Nada: no se carga; sus claves hijas son huérfanas | DV-10 | Reparación explícita, o reconstrucción |
| `Ready` sin vector, o con `output_hash` distinto | `Stale`; no se sirve | DV-12 | `rebuild` |
| `Ready` cuya fuente ya no es la cabeza | `Stale` (estado válido, sin hallazgo) | ninguno | `rebuild` |
| Descriptor de una unidad borrada | Ninguno (su borrado lo retiró) | DV-14 si existe | Reparación explícita |

**Reparación.** La reparación es una llamada explícita de E8-04 que escribe un `ChangeSet` de propósito `Derived`. Abrir nunca repara. Una base sin ningún derivado nunca necesita reparación.

**Límite declarado.** Comprobar el `output_hash` de cada `Ready` al abrir lee todos los vectores persistidos: el coste de apertura es proporcional al volumen de vectores. Es el coste aceptado; se mide en E8-07. Si se vuelve inaceptable, la comprobación pasa a perezosa, y mientras tanto ese cambio es una decisión nueva, no un ajuste.

## Supresión (S5)

ADR-0009, opción 3, aplicada a la clase «vector derivado» (asignación que ADR-0009 A-1 pide por clase; las demás clases siguen sin asignar). El tratamiento sigue la sección 5.3 del acta.

1. **Mismo `ChangeSet`.** Borrar una unidad (revisión `Delete`) o purgar una revisión por retención borra, en la misma transacción, los descriptores, vectores y entradas `0x62` de los derivados de esa fuente. Se encuentran con el rango `0x62 ‖ unit_id` (y, en retención, `0x62 ‖ unit_id ‖ rev_no`).
2. **Lo que se borra al instante** (I-1, extendido a derivados): tras el commit, ningún `get` ni `scan` de `0x60`–`0x62` devuelve las claves borradas.
3. **Instantáneas abiertas** (I-2): una instantánea tomada antes del commit sigue leyendo descriptores y vectores hasta que se suelta.
4. **Lo que sobrevive en disco** hasta la compactación: los bytes de un vector o de un descriptor pueden seguir en un WAL o en una tabla hasta que una compactación cubra su rango. Eso incluye el `unit_id` de la fuente, que está en la clave de `0x60` y de `0x62`. Tras `compact_range` que cubra las claves y un cierre limpio, el centinela de un vector no aparece en ningún archivo vivo que no retenga una instantánea (C-1, C-3). El plazo es el de ADR-0009 (C-4): la compactación, o la liberación de la instantánea, y no una duración.
5. **Sin borrado físico.** El motor no sobrescribe los bloques liberados (ADR-0009, punto 3; `threat-model.md` §6.5).
6. **Sin borrado criptográfico.** Depende de ADR-0006, pendiente (ADR-0009, punto 4). Los vectores persistidos van en claro.
7. **Sin ventana de backup declarada.** Hoy no hay función de copia en el motor (ADR-0009, punto 5; acta D11).
8. **Relaciones.** Borrar una relación no toca estos derivados: su fuente es una revisión de unidad, no una relación. Los derivados con varias fuentes (E8-06) tendrán su propia regla.

**Lo que cambia respecto de E8-03.** Hoy E8-03 marca `stale` a los derivados de una unidad borrada, porque `DerivedChangeSet::commit` trata todas las revisiones del commit igual, y el `delete` de `ChangeSet` añade una revisión de borrado (`db.rs`, `delete` → `members.push`). La regla de este ADR es otra: borrado, no obsolescencia. E8-05 cambia el código; este ADR fija el comportamiento persistido.

## Fuera de alcance

- **Índice aproximado en disco.** La búsqueda sigue siendo exacta (ADR-0011), sobre los vectores que entrega la capa. Un índice aproximado persistido necesita su propio ADR.
- **Modelo en el núcleo, y red.** Ningún paso de esta capa llama a un modelo ni abre una conexión (I-31). El perfil es texto opaco; quien llama produce el vector.
- **Cifrado.** ADR-0006, pendiente. El bit `AEAD` del motor sigue rechazado (`Incompatible("encryption not supported")`).
- **Resúmenes y derivados con varias fuentes**, de E8-06.
- **Borrado físico**, y cualquier promesa sobre el medio.

## Criterios de aceptación de E8-04

E8-04 se acepta si cumple cada uno de estos enunciados, con la prueba que se indica. Ninguno se da por cumplido por lectura del código.

| # | Enunciado comprobable | Prueba |
|---|---|---|
| T1 | Todo valor de §B (descriptor, vector, `0x62`, marca 2) se codifica y decodifica de vuelta byte a byte. Hay vectores comprometidos para cada tipo de registro, positivos y negativos con su regla DV o UNIT | Vectores en `crates/kerno-units/tests/vectors/derived-v2/`, probados por el códec de producción |
| T2 | Un decodificador independiente, escrito solo desde este ADR (§A–§D), coincide con el códec en regla, clase y contenido, sobre los vectores y sobre ediciones de bytes (con CRC recalculado cuando aplique) | `spec_decoder_agrees_derived_v2`, mismo patrón que ADR-0013 |
| T3 | Para todo descriptor, el `derived_id` recalculado (§C.1) es igual a su clave. Cambiar un byte de `source_rev` o de `profile` hace fallar DV-10 y el registro no lo carga | Prueba de vectores y mutación de un byte |
| T4 | Un `output_hash` distinto del hash del valor produce DV-12 y no se sirve el vector | `vector_hash_mismatch_is_not_served` |
| T5 | **Una build sin la característica rechaza al abrir una base con derivados.** Un binario construido desde `d01f646` (último commit antes de E8-04) abre el directorio fixture `derived-v2/db-with-vector` y devuelve `IncompatibleLayer` con la regla UNIT-16 y `found_layer_version = 2`. El árbol de archivos (nombres y hash de cada uno) es idéntico antes y después | Comando sobre el binario de referencia; el resultado se registra en la evidencia de E8-04 |
| T6 | En la suite de E8-04, el mismo fixture, leído con el lector de capa 1 (`LayerMeta::decode` con máximo 1), da UNIT-16 | `layer1_reader_refuses_derived_fixture` |
| T7 | `UnitDb::open` de ese fixture con la build de E8-04 abre y no escribe: el árbol de archivos no cambia | `open_derived_fixture_writes_nothing` |
| T8 | La marca pasa a 2 en la misma transacción que la primera clave `0x60`–`0x62`. Un corte en cada punto de esa transacción deja, tras recuperar, o cero claves derivadas con marca 1, o claves con marca 2 | `crash.rs`: `derived_mark_every_point_{power_loss,process_kill}` |
| T9 | Una base con derivados no vuelve a 1, ni con `set_policy` ni con ningún `ChangeSet` posterior | `mark_never_lowers` |
| T10 | Un corte en cada punto de un `ChangeSet` que actualiza una unidad con un derivado `Ready` deja, tras recuperar, un estado en el que ningún `Ready` tiene una fuente distinta de la cabeza (I-23, I-25, I-38) | `stale_mark_every_point_{power_loss,process_kill}` |
| T11 | Un fallo de sincronización durante ese commit no deja ningún derivado marcado ni ningún vector escrito | `a_commit_that_fails_marks_nothing`, en su versión persistida |
| T12 | Mutación: quitar la marcación de un `ChangeSet` que actualiza una unidad vuelve rojo T10 | Mutación sobre una copia del árbol, nunca sobre el de trabajo |
| T13 | Reconstruir tras un cambio de fuente da el mismo `derived_id`, `input_hash`, `output_hash` y bytes de vector que construir de nuevo sobre las mismas fuentes (I-24) | Comparación byte a byte, como `rebuilt_equals_fresh` |
| T14 | Tras borrar todas las claves `0x60`–`0x62` y reabrir, cada unidad y cada lectura son iguales a las de antes (la verdad no depende de los derivados) | `derived_loss_leaves_units_unchanged` |
| T15 | Cada caso de la tabla de §S4 tiene su prueba: vector sin descriptor, descriptor sin `0x62`, `derived_id` que no cuadra, `Ready` sin vector, `output_hash` distinto, `Ready` con fuente vieja. En la apertura nada se sirve como `Ready` sin comprobación, y `verify_units` informa la regla | `reopen_refuses_or_stales_each_damage_case` |
| T16 | Reabrir un `Ready` cuya fuente cambió lo deja `Stale` antes de servir nada (I-23) | `reopen_after_source_change_is_stale` |
| T17 | Tras un `commit` que borra una unidad con dependientes, `get` y `scan` de `0x60`–`0x62` no devuelven sus claves (I-1, I-42) | `delete_hides_derived_at_once` |
| T18 | Una instantánea tomada antes de ese `commit` sigue leyendo descriptores y vectores hasta que se suelta (I-2) | `snapshot_keeps_derived_until_released` |
| T19 | Tras `compact_range` que cubre las claves, un cierre limpio y sin instantánea retenida, el centinela de un vector no aparece en ningún archivo vivo (C-1, C-3). Las predicciones se escriben antes de ejecutar | `derived_sentinel_survival`, forma de E9A-01 |
| T20 | Un archivado o desarchivado de la fuente marca `Stale` (no borra). Un borrado de la unidad borra sin marcar. Una retención que purga una revisión borra sus derivados en la misma transacción | `archive_is_stale_delete_is_removed` |
| T21 | `supersede` escribe en una transacción el descriptor nuevo y su vector, pasa el antiguo a `Superseded`, borra su vector y conserva su entrada `0x62`, como todo descriptor | `supersede_is_one_changeset` |
| T22 | Escribir un vector cuya fuente no es la cabeza en la transacción se rechaza y no escribe nada | `stale_source_write_is_refused` |
| T23 | Un `ChangeSet` de propósito `Derived` no contiene claves de unidades ni de relaciones (DV-13) | Prueba de verificación con mutación |
| T24 | Los centinelas de I-37 (`tests/sentinels.rs`) cubren los valores de vector y los contenidos de unidades: ninguno aparece en errores, observaciones ni `verify_units` | Centinelas extendidos |
| T25 | Las guardias de vectores del formato del motor (`crates/kerno-storage/tests/vectors/v1/`) pasan sin cambio de un byte, y `crates/kerno-storage/src/format/features.rs` no cambia | `git diff` vacío en esas rutas, y la guardia de CI |
| T26 | `Cargo.toml` y `Cargo.lock` no añaden dependencias. El código no abre ninguna conexión (I-31) | Revisión de dependencias |

## Evidencia

- **E8-03 pasó** (ver [Condición de D2](#condición-de-d2-e8-03-pasó)): 13 y 46 pruebas en verde.
- **Lectura del código** (commit `d01f646`):
  - `crates/kerno-units/src/codec.rs`: las claves `0x40`–`0x42` y `0x50`–`0x52` son las únicas conocidas por encima de `0x3F`; UNIT-2 rechaza el resto del nibble `0x4`–`0xF` con `IncompatibleLayer`.
  - `crates/kerno-units/src/db.rs`: `from_engine` llama a `read_mark`, que decodifica solo la marca `layer` y, si esta no existe pero hay claves, devuelve `NotAUnitDatabase`. Ninguna otra clave se lee al abrir. `begin_changeset_at` escribe la marca en la transacción si falta.
  - `crates/kerno-units/src/derived.rs`: `DerivedDescriptor::id` hashea fuentes, método, versión y parámetros, con `F(·)` little-endian. No hay campo de perfil de modelo ni fecha.
  - `crates/kerno-units/src/derived_stale.rs`: `input_hash` hashea `unit ‖ rev` en little-endian sin prefijo de longitud. `DerivedChangeSet::commit` marca después de `self.cs.commit()?`.
  - `crates/kerno-units/src/types.rs`: `Purpose` es `Application=0, Retention=1, Maintenance=2, Relation=3, Index=4`. `RevisionId` ordena por `(unit, rev)`.
  - `crates/kerno-storage/src/format/features.rs`: `KNOWN_INCOMPAT = AEAD`, `WRITE_INCOMPAT = 0`, `KNOWN_RO_COMPAT = 0`, `KNOWN_COMPAT = 0`. No cambian.
  - `crates/kerno-units/src/assoc_rank.rs`: `VectorSet` tiene una dimensión y devuelve `DimensionMismatch` a un vector de otra.
- **Ningún código se cambió para este ADR.** Las pruebas de E8-04 no existen todavía.

## Discrepancias con el código y con el acta

El código manda, y estas son las discrepancias que el ADR toma en cuenta:

1. **Falta el perfil de modelo.** El acta (§5.4, §7 I-25) y ADR-0018 piden descriptor con perfil; `DerivedDescriptor` no lo tiene y el `id` no lo hashea. Este ADR lo añade (§B.1, §C.1). E8-04 cambia el dominio del `id` a `…v2`.
2. **Falta la fecha del descriptor.** ADR-0018 la lista; el código no la tiene, y con razón (I-24). Este ADR la deja en el `CHANGESET`.
3. **La marcación es posterior al commit.** `DerivedChangeSet::commit` marca después del commit, en memoria. Válido para memoria, no para persistido (§Atomicidad).
4. **Un borrado marca `stale` en vez de borrar.** Lo hace el E8-03 actual (`db.rs` `delete` añade una revisión; `derived_stale.rs` marca todas). ADR-0009 y el acta §5.3 piden borrar. Lo corrige E8-05; este ADR fija el comportamiento persistido.
5. **Dos endianness.** Las claves y valores son big-endian (`units-v1.md` §1); los hashes de identidad e `input_hash` son little-endian. Ambos se mantienen, y este ADR los escribe.
6. **La apertura solo mira la marca.** `read_mark` lee solo `layer`. Por eso la característica tiene que estar en la marca de versión, y no en un registro aparte o en una clave.
7. **`set_policy` y el primer `ChangeSet` escriben `LAYER_VERSION = 1` como constante** (`db.rs` 625 y 1373). Hay que cambiarlos para que nunca bajen la versión (T9).
8. **`DerivedDb::open` recibe la lista de descriptores del llamador.** Con persistencia, la fuente de verdad es la base. La lista deja de ser fuente.
9. **`units-v1.md` dice «Borrador normativo (E3-01)» en su estado**, aunque la acta de cierre de la Etapa 3 registra sus vectores. No se cambia aquí.
10. **ADR-0009 A-1** pide asignar cada clase de dato a una opción, y su propia tabla dice «No cumplido». Este ADR asigna la clase de vectores. Las demás clases siguen abiertas.

## Consecuencias

- **Formato.** La capa 2 es nueva; la capa 1 y el motor no cambian. Una base sin vectores sigue siendo de capa 1 y se abre con cualquier build de capa 1 o 2. Una base con vectores solo se abre con builds de capa 2 o posteriores. Es el rechazo que pide el acta.
- **Motor.** Ninguna línea del formato del motor cambia. Una herramienta que abra el motor directamente, sin la capa, lee las claves como bytes y no rechaza nada. La frontera de compatibilidad es la capa, no el motor.
- **Coste.** Un descriptor por derivado, una entrada `0x62` por descriptor y un vector por `Ready`. La apertura lee todos los vectores para comprobar su hash (§Reconstrucción, límite declarado).
- **Revisar cuando.** E8-07 mide el coste de apertura y del índice inverso. Si el coste del índice inverso es desproporcionado, ADR-0018 manda reabrir. Si la búsqueda necesita índice aproximado, es un ADR nuevo.
- **Compromisos.** Los tags `0x60`–`0x62`, los campos de §B y las reglas DV-1 a DV-13 quedan fijados al aceptarse E8-04. Cambiarlos exige un ADR nuevo y una versión de capa nueva, no una edición.

## Seguimiento

- **E8-04**: implementar §A–§D, cumplir T1–T26, y crear `docs/file-format/units-v2.md`, que copia §A–§D tal cual (el decodificador independiente se escribe desde ese documento). Actualizar `docs/file-format/README.md`.
- **E8-04, primer paso**: fijar el `layer_version` de capa 2 en `LAYER_VERSION` solo cuando la base tiene derivados. La constante 1 de `db.rs` no puede seguir escribiéndose en bases con derivados (T9).
- **E8-05**: sustituir la marcación de borrado por el borrado de §S5 (discrepancia 4).
- **Fuera de este ADR**: ADR-0006 (cifrado), ADR-0009 A-1 (asignación de las demás clases), y la ventana de backup de D11 cuando exista una función de copia.
- **Revisión de ADR-0011**: el propietario decide si su fila pasa a «Reemplazado por ADR-0020» en su parte de persistencia. Este ADR no edita ADR-0011.
</content>
