# Decisión de la condición 1 de ADR-0001 (WP05)

> **Decisión abierta.** T1 sigue sin cumplirse después del paso condicional, y este documento recoge un **candidato** a NO-GO, no un NO-GO. Todas las cifras son de una máquina compartida con otros paquetes (carga de 1 minuto entre 0,7 y 7,4 en las sesiones de WP05) y la dispersión entre ejecuciones de T1 (2,0–3,8 ms) es del orden de la distancia al objetivo. La decisión espera a la repetición de WP14 con la máquina en reposo y la toma el propietario. **Actualización:** la palanca 1 de §3 (WAL preasignado y reciclado) ya está implementada y medida ([§8 del diagnóstico](f1-diagnostico-colas.md#8-preasignación-y-reciclado-del-wal)); T1 sigue sin cumplirse (2,08× fjall y 1,93× btree) y el estado no cambia.

| Campo | Valor |
|---|---|
| Estado | **Candidato a NO-GO, provisional.** Lo decide el propietario; la medición de referencia es la de WP14 con la máquina en reposo |
| Fecha | 2026-10-28 |
| Origen | Regla de decisión de [§5.4 del diseño de la Fase 1](../architecture/phase1-design.md): T1 incumplido por > 25 % ⇒ paso condicional; sigue incumplido ⇒ este documento |
| Datos y análisis | [Resultados de WP05](f1-diagnostico-colas.md#7-resultados-de-wp05), [`spikes/storage/results/2026-10-28-f1-wp05/`](../../spikes/storage/results/2026-10-28-f1-wp05) |
| Objetivos | [f1-objetivos-rendimiento.md](f1-objetivos-rendimiento.md) (T1 y T4) |

## 1. Qué se midió

| Objetivo | Resultado de `kerno` con las palancas por defecto | Objetivo |
|---|---|---|
| T1, `agent_memory` p99 de escritura, frente a fjall | 2,65× (2,43× con subcompactaciones y 2 hilos) | ≤ 2,0× |
| T1, frente a spike-btree | 1,93× (1,77×) | ≤ 1,5× |
| T1 con el WAL preasignado y reciclado (palanca 1; 15 repeticiones de dos sesiones, carga 1,1–1,6) | 2,08× (2,44× sin la palanca) | ≤ 2,0×: **no cumple** |
| T1 con el WAL preasignado, frente a spike-btree | 1,93× (2,27×) | ≤ 1,5×: **no cumple** |
| T4, cierre tras `bulk_load` `medium` | 204 ms con las palancas del cierre (587 ms sin ellas) | ≤ 250 ms: **cumple** |
| T4, cierre tras `bulk_load` `large` | 376 ms (1 270 ms) | ≤ 1 000 ms: **cumple** |

El M0 estaba en 5,8 ms de p99 de escritura (3,71× fjall, 5,36× btree), 759 ms y 3 587 ms de cierre. Las palancas de WP05 reducen el p99 a la mitad (5,1 ms con el perfil `m0` frente a 2,6 ms en las mismas sesiones), pero no alcanzan T1; T4 se alcanza con las palancas del cierre ([§7.4 de los resultados](f1-diagnostico-colas.md#74-t2t5-reportmd-largejsonl-mediana-de-3-repeticiones)). En la sesión final (`results.jsonl`, carga 1,1–1,2) T1 queda en 3,12× fjall y 1,85× btree. Se repitió la carga con la máquina menos cargada (5 repeticiones, carga 1,3–1,7) y se aplicó el paso condicional (subcompactaciones, `max_subcompactions = 4`, y `max_background_compactions = 2`); el efecto del paso condicional (−8 %) está dentro de la dispersión.

## 2. Por qué no alcanza

Según el diagnóstico (§3–4 del [informe de WP04](f1-diagnostico-colas.md)), el 99 % de la cola es el `fdatasync` del WAL, que es lento mientras haya otra escritura pendiente en el disco (la del propio motor o la de otros procesos): 12 ms de p99 con escritura de fondo frente a 4 ms sin ella, y 0,42 ms aislado. Las palancas de WP05 reducen la cantidad y la duración de la escritura de fondo, pero no la eliminan, y ninguna cambia lo que el `fdatasync` tiene que confirmar. La cola que queda es la que paga cualquier motor con un WAL propio sobre este sistema de archivos. fjall, que tiene una cola menor, también escribe menos en segundo plano (`write_amp` 1,6 frente a 3,0 en `bulk_load`); este documento no determina si esa es la causa.

El cierre de `bulk_load` se quedaba en su tramo obligatorio (compactar hasta que la métrica de L0 baje de `l0_trigger`), que con una carga aleatoria reescribe todo L1 por cada archivo de L0 que baja. Sin relajar ese suelo, el cierre toma ahora solo los archivos de L0 que hacen falta, parte esa compactación en subintervalos, no sincroniza por tramos y no empieza una compactación con presupuesto que no vaya a caber en él: 204 ms en `medium` y 376 ms en `large`.

## 3. Siguientes palancas

Por orden de relación entre el efecto esperado y el coste, y todas **por medir** antes de adoptar ninguna:

1. **Reciclado y preasignación del WAL** (diseño §9/§10.2): WAL con tamaño ya asignado y escrito, de modo que su `fdatasync` no confirme cambios de metadatos del diario del sistema de archivos. Ataca directamente el mecanismo de la cola (H3). **Hecho** (`Config::wal_preallocate`, por defecto apagada, sin cambio de formato; [§8 del diagnóstico](f1-diagnostico-colas.md#8-preasignación-y-reciclado-del-wal)). Resultado: el p50 de escritura de `agent_memory` baja un 40 % y los p99 de T2 de LiveSpec, `append_seq` e `history_sim` un 30–40 %; el p99 de T1 mejora un 15 % de mediana, sin separar las distribuciones; el microbanco de `fdatasync` solo muestra ganancia con la máquina cargada por otros procesos. Costes: +1,0 de `write_amp` en `bulk_load`, +13 ms de `reopen_ms` y hasta un WAL más de disco. T1 no se cumple.
2. **Group commit** para los escritores concurrentes (reduce el número de `fdatasync` por commit; no ayuda a un único escritor serial como el de `agent_memory`, así que su efecto en T1 es menor).
3. (Hecho para T4: subcompactaciones solo en el cierre, `close_subcompactions = 8`; las de fondo siguen en `max_subcompactions = 1` porque a 4 no mejoraron T1.)
4. **Reducir la escritura de fondo**: tamaños de nivel dinámicos y compactación por prioridad de bytes; el límite es la amplificación de espacio.
5. **Escribir las SST con E/S directa o con `sync_file_range`** para que el volcado del núcleo no compita con el WAL (dependiente del sistema de archivos; R-S3 del diagnóstico).

## 4. Contingencia (R-10/R-11)

El diseño remite a la contingencia R-10/R-11 (C-33) en este punto, sin detallarla. Se proponen dos caminos para que el propietario elija si confirma el NO-GO tras la repetición de WP14:

- **Seguir con el motor propio** y registrar T1 y T4 como incumplimientos documentados (el cierre de la condición 1 pasa a ser «cola acotada a 2–3× en vez de 11×»), atacando las palancas 2, 4 y 5 de §3 como trabajo de la Fase 2 (la 1 ya está hecha y no basta).
- **O reabrir la elección del motor** (ADR-0001, condición 1) con las cifras de §1: fjall sigue siendo 2,4–2,9× mejor en esta carga a costa de 1,5× de amplificación de espacio, 420 ms de reapertura y 136 MB de memoria máxima residente (`report.md`).

La ejecución secuencial de la Fase 1 continúa mientras tanto, y el formato v1 no depende de esta decisión.

## 5. Límites de esta decisión

- Todas las cifras son de una máquina compartida (carga de 1 minuto entre 0,7 y 7,4) y de un solo sistema de archivos (ext4 sobre un disco virtual). La proporción con fjall varió entre 2,43× y 2,87× según la sesión.
- El paso condicional se midió con 15 repeticiones intercaladas, pero un efecto del 8 % no se distingue de la dispersión entre ejecuciones (2,0–3,8 ms).
- T2 de Cauce (`step_commit_p99_us` 2,31× `lsm`) se atribuyó a los `sync_data` intermedios de `bytes_per_sync` en las compactaciones de fondo: quitarlos lo deja en 1,18× `lsm`, dentro del objetivo, pero sube el p99 de T1 de 2,3 a 3,4 ms. Es un conflicto entre dos objetivos y lo decide el propietario: el valor por defecto se queda en 1 MiB, que favorece T1 ([§7.4 de los resultados](f1-diagnostico-colas.md#74-t2t5-reportmd-largejsonl-mediana-de-3-repeticiones)).
- La medición de la palanca 1 ([§8.3](f1-diagnostico-colas.md#83-agent_memory-y-t1)) tiene la misma limitación (carga de 1 minuto 1,1–2,0; cuartiles del p99 de T1 con la palanca 1 708–2 466 µs): la mejora del 15 % no separa las distribuciones. Su evidencia más firme es el p50 (−40 %), que no es el objetivo T1. Con ella, `module_scan_p99_us` frente a `lsm` pasa de 1,16× a 1,32× en una sesión (10–14 % en tres, dentro de la dispersión y sin explicar), por encima del objetivo de 1,2×.
- `module_scan_p99_us` de LiveSpec cumple frente a `lsm` (0,93×; el 1,24× anterior era ruido con carga 5+) y no frente a btree (2,02×), igual que el M0 (2,18×): no lo causa una palanca de WP05.
