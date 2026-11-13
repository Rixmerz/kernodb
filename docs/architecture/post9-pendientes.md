# Pendientes posteriores a la Etapa 9

| Campo | Valor |
|---|---|
| Fecha | 2026-11-13 |
| Fuente | [Cierre de la Etapa 9](etapa9-cierre.md) §5, los doce puntos, en su orden |
| Qué es | La lista de los puntos abiertos del cierre con su estado actual. Se actualiza por paquete posterior; el cierre no se edita |
| Paquete que la abre | **P1** (K1). Los demás puntos están como los dejó el cierre: «sin cambio» quiere decir que P1 no los tocó, no que se hayan revisado de nuevo |

| # | Punto del cierre | Estado | Nota |
|---|---|---|---|
| 1 | Roadmap, estado de la Fase 1 | Resuelto (2026-11-13, en el cierre) | Sin cambio |
| 2 | K-E3-7: K1, K2, K3, K4, K5 | Ver el desglose siguiente | |
| 3 | Mensaje de `NoValidSources` | Abierto, sin cambio | Fijado por `consolidation.rs:496` |
| 4 | E9-12, estrés de derivados (F-09) | Abierto, parcial, sin cambio | |
| 5 | E9-03, redacción | Abierto, límite declarado, sin cambio | |
| 6 | E9-08g | Abierto, sin cambio | |
| 7 | E9-14, fuzzing (H-7) | Abierto, verificado con límites, sin cambio | |
| 8 | E9-21, ejemplo | Abierto, límite declarado, sin cambio | |
| 9 | E9-20, medición | Abierto, límite declarado, sin cambio | |
| 10 | E9-15, mutaciones de E8 | Abierto, límite declarado, sin cambio | |
| 11 | Prueba posiblemente intermitente (`space_is_reclaimed_after_range_deletes`) | Abierto, sin investigar, sin cambio | |
| 12 | Historia del repositorio | Mancha aceptada, sin cambio | |

## Punto 2, desglose

| Hallazgo | Estado | Qué lo muestra |
|---|---|---|
| **K1** | **Cerrado en P1 desde el extremo que perdió su entrada**, para relaciones con registro de conteo (capa 4). **Abierto, declarado**: relaciones de una base de antes de la capa 4 hasta que se crea o borra una relación de su `(unidad, dirección, tipo)`, y el doble daño (entrada y conteo perdidos juntos) | [etapa9-k-e3-7.md](etapa9-k-e3-7.md) §9; [units-v4.md](../file-format/units-v4.md) §7; `crates/kerno-units/tests/rel_counts.rs`; [post9-mutaciones.md](post9-mutaciones.md) |
| K2 | Abierto, sin cambio: corregido solo para `IDX_PRIMARY` con el conteo intacto; abierto con el conteo rebajado y para `IDX_TYPE` | [etapa9-k-e3-7.md](etapa9-k-e3-7.md) §8 |
| K3 | No se reproduce, sin cambio | `get_unit_checks_oldest_rev` |
| K4 | Abierto, menor, **sin cambio**. P1 no lo cierra | `change_set_info` sigue sin cruzar los miembros con su `REV_META` |
| K5 | Abierto, menor, **sin cambio**. P1 no lo cierra | `get_relation` sigue sin comprobar `created_seq` ni los extremos |

La revisión independiente de E9-16 sigue pendiente, y la de P1 también: el autor de P1 no es su revisor (acta §4.10).

## Efecto de P1 sobre un límite de la Etapa 7

El límite H-1 de [etapa7-cierre.md](etapa7-cierre.md) (R-2): con 64 relaciones o más, una relación creada después y sin ninguna de sus dos entradas de adyacencia no se detectaba en consulta. Con los conteos de P1 **se detecta** (`Fallback` `Corrupt` en el salto) en una base con conteos. La prueba que fijaba el límite, `completeness_is_not_a_query_time_check`, pasa a llamarse `a_relation_lost_with_both_entries_is_caught_by_its_counts` en `crates/kerno-units/tests/assoc_indexed.rs` y afirma lo contrario; el cierre de la Etapa 7 no se edita y su cita queda como el estado de entonces. El mismo límite persiste para relaciones de una base de antes de la capa 4 sin conteo.
