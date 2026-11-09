# E6-00: re-medición del planificador y de la observación

| Campo | Valor |
|---|---|
| Fecha | 2026-11-09 |
| Paquete | E6-00 ([acta de la Etapa 6](etapa6-acta.md) §3) |
| Comando | `cargo run -q --release -p kerno-units --all-features --example index_bench` |
| Máquina | 4 CPU, sin otra carga (load average 0,31 al empezar) |

## Resultado

| Medida | 20 000 unidades | 100 000 unidades | Umbral | Estado |
|---|---|---|---|---|
| Sobrecoste del planificador sobre W1, p50 | −0,1 % | −1,4 % | — | — |
| Sobrecoste del planificador sobre W1, p99 | +5,6 % | +9,7 % | ≤ 10 % | cumple |
| Observación `aggregate` | +0,85 % | −1,18 % | ≤ 5 % de la p99 | cumple |
| Observación `fingerprint` | +0,86 % | +1,16 % | ≤ 5 % | cumple |
| Observación `diagnostic` | +2,70 % | −1,38 % | ≤ 5 % | cumple |

Los dos riesgos de rendimiento heredados del [cierre de la Etapa 5](etapa5-cierre.md) §5 no se reproducen sobre máquina sin carga: se **cierran**. El p99 de 100 000 unidades sigue siendo ruidoso entre rondas (8–29 ms), así que +9,7 % se lee como «dentro, con poco margen», no como un sobrecoste estable.

## Consecuencia para la decisión 1 de la acta

El planificador no cuesta nada medible en p50. Un plan reutilizable (opción a) solo ahorraría ese coste, de modo que no puede alcanzar la mejora de §6 (p50 ≥ 20 %). Se **confirma la opción (b)**: caché de resultados por huella y `commit_seq`.

## Salida cruda

```text
scale,query,path,p50_ms,p95_ms,p99_ms,mean_hits,build_ms
20000,type,reference,73.167,90.366,107.446,199,110
20000,type,primary,5.967,6.203,6.648,199,110
20000,type,ideal,0.777,0.946,3.681,199,110
20000,type,secondary,0.898,1.064,1.475,199,110
# 20000 type: primary/ideal p50 = 7.68x, primary p95 = 6.203 ms, secondary/ideal p50 = 1.16x -> threshold not exceeded
# round 0: planner p50/p99 = 0.931/1.269 ms, legacy = 0.915/1.393 ms
# round 1: planner p50/p99 = 0.925/1.679 ms, legacy = 0.896/1.140 ms
# round 2: planner p50/p99 = 0.909/2.044 ms, legacy = 0.905/1.341 ms
# round 3: planner p50/p99 = 1.140/1.562 ms, legacy = 0.899/1.800 ms
# round 4: planner p50/p99 = 0.869/1.204 ms, legacy = 0.870/1.267 ms
# 20000 type: planner overhead over the current path: p50 -0.1 %, p99 +5.6 % (threshold 10 %) -> within
# 20000 type: observation at Aggregate adds p50 +0.0093 ms = +0.85 % of the unobserved p99 1.092 ms (threshold 5 %) -> within
# 20000 type: observation at Fingerprint adds p50 +0.0093 ms = +0.86 % of the unobserved p99 1.092 ms (threshold 5 %) -> within
# 20000 type: observation at Diagnostic adds p50 +0.0295 ms = +2.70 % of the unobserved p99 1.092 ms (threshold 5 %) -> within
20000,type+schema,reference,74.392,104.175,110.658,67,110
20000,type+schema,primary,5.323,5.849,8.010,67,110
20000,type+schema,ideal,0.274,0.339,0.383,67,110
20000,type+schema,secondary,0.294,0.368,0.409,67,110
# 20000 type+schema: primary/ideal p50 = 19.40x, primary p95 = 5.849 ms, secondary/ideal p50 = 1.07x -> threshold not exceeded
100000,type,reference,427.774,483.378,518.275,999,522
100000,type,primary,46.782,63.543,68.357,999,522
100000,type,ideal,6.162,7.256,8.282,999,522
100000,type,secondary,7.329,10.130,12.873,999,522
# 100000 type: primary/ideal p50 = 7.59x, primary p95 = 63.543 ms, secondary/ideal p50 = 1.19x -> threshold exceeded
# round 0: planner p50/p99 = 7.502/13.660 ms, legacy = 7.091/10.422 ms
# round 1: planner p50/p99 = 6.857/8.880 ms, legacy = 6.956/10.071 ms
# round 2: planner p50/p99 = 7.324/29.368 ms, legacy = 7.458/19.011 ms
# round 3: planner p50/p99 = 7.608/20.782 ms, legacy = 7.506/13.210 ms
# round 4: planner p50/p99 = 7.197/9.707 ms, legacy = 6.954/8.097 ms
# 100000 type: planner overhead over the current path: p50 -1.4 %, p99 +9.7 % (threshold 10 %) -> within
# 100000 type: observation at Aggregate adds p50 -0.0938 ms = -1.18 % of the unobserved p99 7.973 ms (threshold 5 %) -> within
# 100000 type: observation at Fingerprint adds p50 +0.0923 ms = +1.16 % of the unobserved p99 7.973 ms (threshold 5 %) -> within
# 100000 type: observation at Diagnostic adds p50 -0.1099 ms = -1.38 % of the unobserved p99 7.973 ms (threshold 5 %) -> within
100000,type+schema,reference,422.397,472.190,582.915,333,522
100000,type+schema,primary,39.227,55.550,59.859,333,522
100000,type+schema,ideal,2.186,2.506,2.659,333,522
100000,type+schema,secondary,2.540,3.402,4.977,333,522
# 100000 type+schema: primary/ideal p50 = 17.94x, primary p95 = 55.550 ms, secondary/ideal p50 = 1.16x -> threshold exceeded
# secondary index justified by W1: true
exit=0
```
