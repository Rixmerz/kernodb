# Etapa 4 — Workloads de los índices secundarios (E4-04)

Este documento fija, **antes de medir**, los workloads que pueden justificar un índice secundario ([acta de la Etapa 4](etapa4-acta.md) §3, E4-04): «solo los que un workload medido justifique; cada uno tiene su workload escrito antes de construirse». La metodología es la de [ADR-0012](../adr/0012-metodologia-de-benchmarks.md) (mismo proceso, mismas opciones, `KernoDb` sobre el sistema de archivos real, calentamiento separado, mediana y percentiles). Los resultados se añaden en la §4 sin cambiar las §1–§3.

## 1. Qué consultas soporta la etapa

La única consulta filtrada de la Etapa 4 es `UnitFilter` (E4-03): unidades **activas** con `unit_type` igual a un valor, `schema_version` igual a un valor, o ambas, en orden de id. La ruta primaria (`IDX_PRIMARY`) recorre **todas** las entradas activas, aplica el filtro sobre la entrada y abre (lectura verificada) solo las que coinciden. Un índice secundario por `(unit_type, schema_version)` evitaría recorrer las entradas que no coinciden; no puede evitar abrir las que sí. Las relaciones ya tienen sus índices de adyacencia (`REL_OUT`/`REL_IN`, ADR-0014) y no entran aquí.

## 2. Workload W1 — filtro selectivo por tipo

| Parámetro | Valor |
|---|---|
| Unidades activas | `N` = 20 000 y 100 000 (escala `small` y medio camino a `medium` de ADR-0012) |
| Unidades archivadas | 10 % adicional (no aparecen en el índice ni en la respuesta) |
| Tipos | 100 tipos uniformes → **selectividad 1 %** por tipo (200 y 1 000 resultados) |
| Versiones de esquema | 1–3 uniformes; la consulta combinada `(tipo, versión)` tiene selectividad ≈0,33 % |
| Carga útil | 256 bytes; metadatos vacíos |
| Consultas | `query(tipo)` y `query(tipo, versión)` con el índice `Ready`; 10 % de calentamiento no medido; 200 muestras (`N` = 20 000) y 100 (`N` = 100 000) por ruta |
| Rutas medidas | referencia (`reference_filter`), primaria (`query`, `QuerySource::Index`) y **cota ideal**: `get_unit` de exactamente los ids que coinciden, sin recorrido. Es lo mínimo que puede costar cualquier índice secundario, que además tendría que recorrer su propio rango |

## 3. Umbral (fijado antes de medir)

Un índice secundario sobre `(unit_type, schema_version)` está justificado **solo si**, en `N` = 100 000 y para alguna de las dos consultas, se cumplen **las dos** condiciones:

1. **Margen**: la mediana de la ruta primaria supera **2×** la mediana de la cota ideal (un índice podría, como mucho, dividir la latencia por dos o más).
2. **Importa**: el p95 de la ruta primaria supera **10 ms**.

Si no se cumplen, no se construye ningún índice secundario y la conclusión se documenta con las cifras. Ningún umbral se rebaja después de medir.

## 4. Resultados (2026-11-05)

Comando: `cargo run --release -p kerno-units --example index_bench` (3 pasadas, semilla 42). Entorno: 4 vCPU Intel Xeon @ 2,80 GHz, 15 GiB, Linux 6.18, `rustc` 1.97.0, caché del SO caliente. Datos crudos en [`spikes/storage/results/2026-11-05-e4-04/`](../../spikes/storage/results/2026-11-05-e4-04/) (`index_bench-w1-before.txt`: antes de construir el índice; `-after.txt`: con él). Cifras en ms, mediana de las 3 pasadas de cada percentil.

### 4.1 Antes del índice secundario (ruta primaria de E4-03)

| `N` | Consulta | Resultados | Referencia p50 | Primaria p50 / p95 | Cota ideal p50 | Primaria / ideal (p50) |
|---|---|---|---|---|---|---|
| 20 000 | tipo | 199 | 93,3 | 9,9 / 12,6 | 1,51 | 7,4× |
| 20 000 | tipo + versión | 67 | 100,9 | 6,9 / 12,0 | 0,54 | 14,4× |
| 100 000 | tipo | 999 | 508,7 | 59,4 / 68,7 | 12,15 | 4,9× |
| 100 000 | tipo + versión | 333 | 520,9 | 41,2 / 59,1 | 4,50 | 10,6× |

En `N` = 100 000 las dos consultas incumplen el umbral en las 3 pasadas: la ruta primaria es 4,8–12,3× la cota ideal (> 2×) y su p95 es 57–71 ms (> 10 ms). Recorrer las 100 000 entradas de `IDX_PRIMARY` cuesta más que abrir las 333–999 unidades que coinciden. **El workload justifica un índice secundario por `(unit_type, schema_version)`**, que se construye: `IDX_TYPE` ([units-v1.md](../file-format/units-v1.md) §3.11).

### 4.2 Con `IDX_TYPE`

| `N` | Consulta | Primaria p50 | `IDX_TYPE` p50 / p95 | Cota ideal p50 | `IDX_TYPE` / ideal (p50) |
|---|---|---|---|---|---|
| 20 000 | tipo | 10,2 | 1,49 / 2,16 | 1,25 | 1,07–1,19× |
| 20 000 | tipo + versión | 8,2 | 0,49 / 0,71 | 0,44 | 1,07–1,14× |
| 100 000 | tipo | 62,3 | 13,1 / 17,1 | 10,18 | 1,25–1,51× |
| 100 000 | tipo + versión | 55,6 | 5,2 / 6,3 | 3,93 | 1,26–1,31× |

Con el índice, la consulta por tipo queda a 1,07–1,51× de la cota ideal (4–11× más rápida que la ruta primaria); la de tipo y versión, a ≤ 1,31×. En `N` = 100 000 el p95 por tipo sigue por encima de 10 ms (17 ms) porque solo abrir los 999 resultados ya cuesta 12 ms (cota ideal): ningún índice puede bajar de ahí, y el coste restante es la verificación de cada unidad devuelta.

**Coste**: la construcción escribe el doble de entradas. Su duración pasa de 54–75 ms a 134–179 ms con 20 000 unidades y de 317–360 ms a 793–858 ms con 100 000 (≈2,4×). Ningún `ChangeSet` de aplicación paga nada: el índice es derivado (ADR-0007) y solo lo escribe `rebuild_index`.

### 4.3 Conclusión

W1 justifica `IDX_TYPE` y es el único índice secundario de la etapa. Un filtro solo por versión de esquema no tiene workload escrito y sigue por `IDX_PRIMARY`.
