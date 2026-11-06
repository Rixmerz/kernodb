# Etapa 5 — Huella normalizada y plan explicable (E5-01)

Contrato de E5-01 ([acta de la Etapa 5](etapa5-acta.md) §3, decisiones de §8; [ADR-0015](../adr/0015-privacidad-de-la-observabilidad.md), aceptado con la evidencia de E5-04). El código está en `crates/kerno-units/src/fingerprint.rs` y `plan.rs`. Este paquete **no cambia ninguna consulta**: `query` y `relations_of` se comportan igual; el planificador que usa estos tipos es E5-02.

## 1. Huella

Una consulta se reduce a su **forma**: qué condiciones fija, con cada literal sustituido por un marcador tipado.

| Consulta | Forma |
|---|---|
| `UnitFilter` sin condiciones | `units[]` |
| `UnitFilter { unit_type, schema_version }` | `units[unit_type=?text,schema_version=?u32]` (solo las condiciones fijadas, en ese orden) |
| Relaciones de una unidad, saliente | `relations[out:unit=?unit_id]` |
| Con tipo de relación, entrante | `relations[in:unit=?unit_id,relation_type=?text]` |

La dirección es estructura (dos valores posibles), no un literal. Los marcadores son `?text`, `?u32` y `?unit_id`.

La huella es el hash **con clave** de la forma, con algoritmo y versión registrados:

```text
BLAKE3-keyed(clave, "kerno.query-fingerprint.v1" ‖ 0x00 ‖ forma), primeros 16 bytes
```

Se muestra como `units/blake3-keyed/v1/<32 dígitos hex>`. Cambiar el algoritmo, la etiqueta, el truncado o la gramática de formas exige subir `FINGERPRINT_VERSION`; los vectores fijos de `tests/fingerprint.rs` lo detectan.

**Clave** (acta §8, decisión 4). `FingerprintKey` es local al proceso y no sale de él: no tiene acceso a sus bytes, `Debug` los oculta, no se serializa y no es comparable con `==` (un secreto no recibe una comparación de tiempo variable; se comparan huellas). `generate()` lee 32 bytes de `/dev/urandom`; `from_bytes` existe para pruebas reproducibles. Dos huellas solo son comparables bajo la misma clave, que es lo buscado: una forma tiene pocos valores posibles y un hash sin clave caería a un ataque de diccionario.

**Qué se muestra en cada nivel** (ADR-0015): la huella (`Fingerprint`) es lo que llevan los niveles `aggregate` y `fingerprint`. El texto de la forma (`NormalizedQuery::text`) no contiene literales, pero es estructura de la consulta: solo el nivel `diagnostic`.

**Vectores.** Los resúmenes de `tests/fingerprint.rs` (clave `00 01 … 1f`, ocho formas) se calcularon con una implementación independiente de BLAKE3 escrita desde la especificación, validada contra el vector oficial del modo con clave (entrada vacía, clave `whats the Elvish word for friend`), y no se leyeron de este código. Ambas implementaciones coinciden.

## 2. Plan y explicación

Un `Plan` dice cómo se responderá una consulta y por qué; **es** la explicación (no hay una segunda estructura que mantener de acuerdo). Contiene la huella de la consulta, nunca la consulta: ni literales ni texto de forma.

| Consulta | Candidatos (orden de desempate) |
|---|---|
| Unidades | `UnitsReference`, `IndexPrimary`, `IndexType` |
| Relaciones | `RelationsReference`, `Adjacency` |

Son solo los caminos que ya existen (acta §2.2). Cada candidato lleva su coste estimado y si es utilizable (`Usable` o `Unusable(IndexNotReady(estado) | NoTypeFilter)`). El plan guarda además el candidato elegido, el motivo (`LowestCost` o `Fallback { index }`) y el estado de frescura del índice que vio la decisión (I-23; `None` en consultas de relaciones, cuyas entradas de adyacencia escribe el mismo conjunto de cambios que la relación, ADR-0014).

`Plan::new` rechaza lo que no se puede explicar con verdad:

1. los candidatos son **exactamente** los de la tabla, una vez y en orden;
2. una consulta de unidades lleva estado de índice y una de relaciones no;
3. **ningún índice no `Ready` se declara utilizable** (I-23): con el índice `Stale`, `Building` o `Failed`, ambos caminos de índice son `Unusable(IndexNotReady(ese estado))`; con `Ready`, solo `IndexType` puede ser inutilizable, y solo por `NoTypeFilter`;
4. los caminos de referencia y de adyacencia son siempre utilizables (son el fallback, I-32), así que siempre hay una respuesta;
5. la elección es el candidato utilizable de menor coste y, en empate, el primero de la tabla; el motivo se **deriva** (`Fallback` si el índice no estaba `Ready`, `LowestCost` si lo estaba), no se recibe.

`Cost` es una cantidad relativa (entradas que el camino leería), comparable solo dentro de un plan. Cómo se estima, con qué estadísticas y la generación de candidatos son de E5-02; este paquete no añade ninguna estadística.

## 3. Pruebas y alcance

`tests/fingerprint.rs` (7 pruebas) y `tests/plan.rs` (10 pruebas): vectores fijos, ausencia de literales centinela en forma, huella y plan, clave, listado completo de candidatos, desempate, fallback con cada estado de frescura, cada rechazo de `Plan::new` y un caso sobre una base real (`Stale` y luego `Ready`).

No cubre: la prueba de centinelas en toda salida de observabilidad y error en cada nivel (E5-04), la elección real de caminos y la prueba diferencial frente a la referencia (E5-02), ni registro alguno de observaciones (E5-03).

ponytail: sin `Display` del plan (los campos son la explicación estructurada; un texto estable lo fijará E5-03/E5-04 cuando haya un consumidor); la clave no se borra al soltarse y `generate()` solo existe en Unix. Ruta de mejora: `getrandom` y `zeroize` si la clave sale de un único proceso de confianza.
