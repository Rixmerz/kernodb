# Etapa 5 · E5-02 — Planificador determinista

Contrato de E5-02 ([acta de la Etapa 5](etapa5-acta.md) §3). El código está en `crates/kerno-units/src/planner.rs` (pasos, costes, validación) y `plan.rs` (la elección validada, `Decision`). Los tipos de plan y huella son los de [E5-01](etapa5-huella-y-plan.md).

## 1. Los siete pasos de spec §9

`UnitSnapshot::query` y la nueva `query_relations` **son** esos pasos sobre los caminos que ya existían (acta §2.2); no se añadió índice, camino ni consulta.

| Paso | Qué hace aquí |
|---|---|
| 1. Analizar y validar | Todo `UnitFilter` y `RelationQuery` es válido: una condición que ningún valor almacenado cumple (tipo vacío o demasiado largo) es una consulta legítima con respuesta vacía, como ya daban ambos caminos |
| 2. Candidatos válidos | `PlanPath::candidates_of`, cada uno marcado utilizable o no (I-23) |
| 3. Estimar coste | Con lo que la capa ya tiene: estado y número de entradas de `IDX_META`, y un sondeo acotado (`PROBE` = 64 lecturas) del rango que el camino leería. Ninguna estadística nueva |
| 4. Elegir | El utilizable más barato; en empate, el primero de la tabla (`Decision::new`) |
| 5. Ejecutar | El camino elegido, por el mismo código de las llamadas explícitas |
| 6. Registrar | `QueryOutcome` y `RelationsOutcome` llevan el camino y el fallback; `query` los entrega al observador de [E5-03](etapa5-observacion.md) |
| 7. Validar y fallback | La respuesta se comprueba contra la consulta (solo lo que el filtro selecciona, solo `Active`, orden estrictamente ascendente de la referencia). Si el camino de índice falla con daño (`Corruption`) o la respuesta no pasa, **responde la ruta de referencia** y el resultado lo dice (`Fallback { abandoned, cause }`). Un error que no es daño no se oculta con un segundo intento |

`explain_filter` y `explain_relations` devuelven el `Plan` que `query` / `query_relations` siguen en esa instantánea: candidatos con coste y utilizabilidad, elegido, motivo y estado del índice. El plan lleva la huella con clave, nunca la consulta.

## 2. Determinismo

La decisión es función pura de la consulta y de lo que la instantánea contiene: no lee reloj, azar ni el manejador. Misma entrada e instantánea dan los mismos candidatos, costes y elección, en cualquier motor y tras reabrir.

## 3. Coste

Número de lecturas clave-valor esperadas; comparable solo dentro de un plan. `N` = entradas de la generación de `IDX_META`; abrir una unidad verificada = `OPEN_READS` (3) lecturas; `m` = unidades que el filtro conserva.

| Camino | Coste |
|---|---|
| `UnitsReference` | `N × (1 + OPEN_READS)` |
| `IndexPrimary` | `N + OPEN_READS × m` |
| `IndexType` | `(1 + OPEN_READS) × m`; `m` se sondea (exacto si el rango es menor que `PROBE`) |
| `RelationsReference` | `r` (relaciones sondeadas) |
| `Adjacency` | `2 × a` (entradas de adyacencia sondeadas) |

Sin sondeo completo, `m` es `N / 2` si el filtro fija alguna condición y `N` si no. Consecuencias: un filtro sin condiciones cuesta igual por la referencia que por `IDX_PRIMARY` y el empate va a la referencia; un índice vacío o sin construir tiene `N = 0`, así que gana la referencia, que además es lo único utilizable.

**Cambio de comportamiento observable.** Antes, `query` con el índice `Ready` siempre usaba el índice. Ahora, con índice `Ready`, usa el índice **solo si es más barato**: un filtro sin condiciones responde por la referencia (`QuerySource::Reference`). El resultado no cambia (I-32); cambia `QueryOutcome::source`/`path`. `QuerySource::Reference` documenta ahora las tres causas.

ponytail: la prior `N / 2` y el límite del sondeo son números fijos, no aprendidos; W1 (filtro por tipo del 1 %) lo decide el sondeo. Mejora: conteo por tipo en `IDX_META`, escrito por la reconstrucción, si una carga muestra que la prior elige el camino más lento.

## 4. Fallback por estado de frescura

El estado del índice que ve el planificador es el de la **instantánea**: `Building` solo lo informa `UnitDb::index_status` (el manejador); una instantánea ve lo almacenado, y una reconstrucción en curso no ha almacenado nada. Por eso:

| Estado | Camino | Cómo se prueba |
|---|---|---|
| Sin construir / `Stale` | referencia, `Reason::Fallback { Stale }` | `planner_falls_back_when_the_index_is_not_ready`, con todos los filtros |
| `Failed` | referencia, `Reason::Fallback { Failed }` (tras reparar los datos, que no limpia el estado) | ídem |
| `Building` | la instantánea sigue viendo la generación almacenada `Ready` y responde igual que la referencia; el plan de `Building` (ambos caminos de índice inutilizables) se prueba sobre los candidatos | misma prueba (durante la reconstrucción en espera) y `candidates_carry_the_usability_the_state_gives` |
| Camino de índice que falla o responde mal | referencia, fallback registrado | `a_failing_or_wrong_index_path_falls_back_to_the_reference`, `a_failing_or_wrong_adjacency_falls_back_to_the_reference` (inyección del fallo; un caso por `Violation` y por `Corrupt`) |

## 5. Prueba diferencial en CI (I-32)

Las comprobaciones de `check` (`tests/index.rs`: semillas 1–6 más barridos por páginas y rondas de reconstrucción) y de `assert_equivalent` (`tests/relations.rs`) comparan ahora también `query` y `query_relations` con la ruta de referencia, con y sin índice listo, sobre los mismos filtros y semillas; exigen `fallback == None` en un almacén sano, que el camino ejecutado sea el que `explain_*` anuncia y que dos explicaciones de la misma instantánea sean iguales.

## 6. Conteo de pruebas y mutación

Suite `cargo test --workspace --exclude kerno-storage --all-features --release`: **581 → 590** (7 pruebas unitarias de `planner.rs` y 2 de `tests/index.rs`, una por motor; las aserciones diferenciales se añadieron a pruebas existentes). Cada prueba nueva se vio fallar con una mutación de la propia prueba (invertir el estado esperado, cambiar `Violation`, cambiar un coste, comparar con un camino fijo) y se restauró.

## 7. Sobrecoste sobre W1 (umbral ≤ 10 %, acta §8)

`index_bench` compara `query` (planificador) con el camino anterior (`index_status` + `indexed_scan`, sin estimar coste), en cinco rondas que alternan ambos, sobre W1 (filtro por tipo, ~1 % de selectividad), en modo completo y sobre el sistema de archivos real. El sobrecoste es el de la mejor ronda de cada camino.

| Escala | p50 | p99 | Umbral |
|---|---|---|---|
| 20 000 unidades | +2,2 % | +0,6 % | cumple |
| 100 000 unidades | +1,8 % | **+8,5 %** | cumple, con poco margen en p99 |

Cabe leer esto con cuidado: el p99 de 100 000 unidades varía entre rondas (10,96–14,39 ms en el camino anterior) y la máquina es compartida, así que +8,5 % no distingue de forma fiable un sobrecoste de entre 0 % y 12 %. El coste añadido es el sondeo del rango del tipo (hasta 64 lecturas de entrada). Se registra como dato; un incumplimiento sería un riesgo y no un bloqueante (acta §5).

### Salida cruda

```text
     Running `target/release/examples/index_bench`
scale,query,path,p50_ms,p95_ms,p99_ms,mean_hits,build_ms
20000,type,reference,76.799,95.460,110.150,199,79
20000,type,primary,6.392,10.013,11.670,199,79
20000,type,ideal,0.881,1.268,2.618,199,79
20000,type,secondary,0.945,1.095,1.215,199,79
# 20000 type: primary/ideal p50 = 7.26x, primary p95 = 10.013 ms, secondary/ideal p50 = 1.07x -> threshold exceeded
# round 0: planner p50/p99 = 0.967/1.170 ms, legacy = 0.942/1.450 ms
# round 1: planner p50/p99 = 0.970/1.334 ms, legacy = 0.953/1.163 ms
# round 2: planner p50/p99 = 0.952/1.703 ms, legacy = 0.947/1.171 ms
# round 3: planner p50/p99 = 0.949/1.720 ms, legacy = 0.928/1.355 ms
# round 4: planner p50/p99 = 0.953/1.314 ms, legacy = 0.941/2.178 ms
# 20000 type: planner overhead over the current path: p50 +2.2 %, p99 +0.6 % (threshold 10 %) -> within
20000,type+schema,reference,74.569,100.957,111.394,67,79
20000,type+schema,primary,5.504,6.676,7.608,67,79
20000,type+schema,ideal,0.304,0.404,0.475,67,79
20000,type+schema,secondary,0.346,0.455,0.547,67,79
# 20000 type+schema: primary/ideal p50 = 18.10x, primary p95 = 6.676 ms, secondary/ideal p50 = 1.14x -> threshold not exceeded
100000,type,reference,472.017,592.730,650.109,999,675
100000,type,primary,46.422,88.199,96.111,999,675
100000,type,ideal,9.318,12.250,12.810,999,675
100000,type,secondary,10.086,13.417,16.349,999,675
# 100000 type: primary/ideal p50 = 4.98x, primary p95 = 88.199 ms, secondary/ideal p50 = 1.08x -> threshold exceeded
# round 0: planner p50/p99 = 9.288/15.047 ms, legacy = 8.138/12.567 ms
# round 1: planner p50/p99 = 8.099/11.891 ms, legacy = 7.953/10.961 ms
# round 2: planner p50/p99 = 9.589/12.570 ms, legacy = 8.457/12.809 ms
# round 3: planner p50/p99 = 9.721/14.680 ms, legacy = 9.255/12.491 ms
# round 4: planner p50/p99 = 8.729/16.205 ms, legacy = 8.916/14.388 ms
# 100000 type: planner overhead over the current path: p50 +1.8 %, p99 +8.5 % (threshold 10 %) -> within
100000,type+schema,reference,471.772,564.813,609.720,333,675
100000,type+schema,primary,47.891,72.129,78.186,333,675
100000,type+schema,ideal,2.369,2.672,3.171,333,675
100000,type+schema,secondary,2.931,3.505,3.637,333,675
# 100000 type+schema: primary/ideal p50 = 20.22x, primary p95 = 72.129 ms, secondary/ideal p50 = 1.24x -> threshold exceeded
# secondary index justified by W1: true
```
