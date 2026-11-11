# Etapa 6 · E6-04 — Medición y centinelas

Contrato de E6-04 ([acta de la Etapa 6](etapa6-acta.md) §3, umbrales de §6, decisiones 2, 4 y 5 de §8). Antecedentes: la medición base y el método, en [E6-00](etapa6-e600-remedicion.md); el uso de la caché en el planificador, en [E6-02](etapa6-integracion.md); la reversibilidad, en [E6-03](etapa6-reversibilidad.md). Garantías: [ADR-0016](../adr/0016-adaptacion-e-invalidacion.md) G7 y [ADR-0015](../adr/0015-privacidad-de-la-observabilidad.md) (I-37).

E6-04 no cambió código de producción, ni el formato persistente, ni el motor; **E6-04b** (§3b) sí toca `cache.rs` y `planner.rs` (solo la decisión de guardar y la huella), sin tocar el formato persistente ni el motor. Añade una sección al banco de pruebas existente (`crates/kerno-units/examples/index_bench.rs`, opción `--cache`), las pruebas `crates/kerno-units/tests/cache_sentinels.rs`, la salida cruda y este documento.

## Resultado en una línea

**E6-04b (optimización del camino de fallo): con la mediana de las rondas, que es el criterio de este paquete, se cumplen los tres umbrales: peor caso sin repeticiones −2,4 % (20 000) y −2,8 % (100 000), p99 de W1 +1,3 %, p50 de W1 −97,9 %.** Con la **mejor ronda** el peor caso a 20 000 unidades sigue fuera (+9,7 %); la medición no separa «dentro» de «fuera» con 5 rondas (§7). La medición original de E6-04 (antes de la optimización) se conserva en §3 como «antes»: el peor caso incumplía (+20,9 % mejor ronda, +11,7 % mediana) y el p99 de W1 cumplía solo en la mejor ronda (+2,0 %; mediana +7,8 %). No se ha tocado ningún umbral. La caché sigue **desactivada por defecto** (acta §8.2). Los centinelas y las pruebas de la caché siguen en verde (§7).

## 1. Método

| Campo | Valor |
|---|---|
| Fecha | 2026-11-09 (E6-04); 2026-11-11 (E6-04b, salida actual) |
| Comando | `cargo run -q --release -p kerno-units --all-features --example index_bench -- --cache > docs/architecture/etapa6-medicion-salida.txt` |
| Máquina | 4 CPU, sin otra carga: load average 0,42 al empezar la medición completa y 1,14 al terminar (`uptime`; la propia corrida es de un hilo); durante la corrida no se lanzó nada más (las mutaciones de §4 se hicieron después) |
| Motor | `KernoDb` sobre disco (`TempDir` del banco), 100 tipos de unidad, `PAYLOAD` y generador de datos de E6-00; semilla 42 |
| Escalas | 20 000 y 100 000 unidades (las de E6-00) |
| Medida | latencia de cada `snapshot.query(filter)` (`Instant`), en ms. Las escrituras y las reconstrucciones del índice **no** se cronometran |
| Calentamiento | el primer 10 % de las lecturas de cada corrida no cuenta |
| Rondas | 5 por comparación; en cada ronda se corre la secuencia **una vez con la caché apagada y otra encendida**, y el orden se invierte cada ronda (apagada→encendida, encendida→apagada, …) para repartir deriva térmica y de caché de página |
| Estadística | p50, p95 y p99 por corrida. La comparación se da de dos formas: **mejor ronda** (el método de E6-00: el mínimo de cada modo) y **mediana de las rondas**. Se informan las dos porque el p99 es ruidoso entre rondas y una sola lectura sería elegir |
| Controles | cada lectura compara sus ids con el resultado esperado (si caché y referencia discrepan, la corrida se detiene); con la caché apagada, `stats()` debe quedar a cero y ninguna respuesta puede salir de ella; en la carga sin repeticiones, no puede haber más lecturas servidas que las recurrencias de la única entrada que se conserva (§3b, punto 3) |

### Las cargas

- **W1** (acta §8, decisión 5): bloques de **una escritura y cuatro lecturas de la misma consulta**. El 20 % de las operaciones son escrituras intercaladas y el 80 % lecturas. La escritura actualiza una unidad existente (no cambia el tamaño de los conjuntos) y mueve `commit_seq`, de modo que la primera lectura tras cada escritura **nunca** puede servirse (la entrada depende exacta de `commit_seq`, G4): de las cuatro lecturas, tres repiten y una no. Así, el 75 % de las lecturas (60 % de las operaciones) son repeticiones servibles; es el máximo que permite una escritura cada cinco operaciones, no un valor elegido. La consulta de cada bloque es `type = ?` con un tipo distinto del anterior (37 es primo con 100).
  - `w1-index-ready`: tras cada escritura se reconstruye el índice (sin cronometrar), de modo que todas las lecturas pueden usarlo, como en la base de E6-00. Es la carga que se juzga. 400 bloques → 1 440 lecturas cronometradas por corrida.
  - `w1-index-stale`: el índice se deja `Stale` tras la primera escritura, así que las lecturas sin caché son la ruta de referencia (≈ 90 ms a 20 000 unidades). Es una carga **complementaria** (la caché gana mucho más porque lo que ahorra es un recorrido completo); no se usa para decidir. 150 bloques → 540 lecturas por corrida.
  - W1 corre solo a 20 000 unidades: una reconstrucción de índice por bloque a 100 000 (≈ 0,5 s) multiplicaría la corrida sin aportar otra escala de decisión. **Es un límite de la medición**: W1 a 100 000 no se ha medido.
- **Sin repeticiones** (peor caso, §6): una lectura por bloque, sin escrituras, recorriendo los 100 tipos con paso 37. La caché guarda una entrada por forma de consulta (`type = ?`), así que cada lectura cae sobre la entrada de **otro** tipo: nunca acierta, la sustituye (`Missed{OtherQuery}` y `Deleted{Replaced}`) y paga todo el camino de la caché: huella, `lookup`, y guardar las filas. 4 000 lecturas a 20 000 unidades (3 600 cronometradas por corrida), 1 500 a 100 000 (1 350).

## 2. Salida cruda

- [`etapa6-medicion-salida.txt`](etapa6-medicion-salida.txt): la salida completa del comando de arriba, sin editar, con la línea `exit=0` y el `uptime` final (las dos últimas líneas las añade el comando que la guarda). Una fila por corrida: `cache,escala,carga,modo,ronda,lecturas_cronometradas,servidas_por_la_caché,p50_ms,p95_ms,p99_ms`; las líneas `#` son las comparaciones.
- Reproducción rápida (humo, 2 000 unidades): `cargo run -q --release -p kerno-units --all-features --example index_bench -- --cache --quick`.

## 3. Resultado frente a los umbrales (§6)

Porcentajes de la caché encendida sobre la apagada (negativo = más rápida). «Mejor» es la mejor ronda de cada modo; «mediana», la ronda mediana.

| Umbral (acta §6) | Carga y escala | Mejor ronda (antes → ahora) | Mediana (antes → ahora) | Estado ahora |
|---|---|---|---|---|
| W1, p50 ≥ 20 % mejor | `w1-index-ready`, 20 000 | −97,7 % → −98,0 % | −97,6 % → −97,9 % | **cumple** |
| W1, p99 ≤ +5 % | `w1-index-ready`, 20 000 | +2,0 % → +4,7 % | +7,8 % → +1,3 % | **cumple** (mejor ronda y mediana) |
| Peor caso, p99 ≤ +5 % | sin repeticiones, 20 000 | +20,9 % → +9,7 % | +11,7 % → −2,4 % | **cumple en la mediana; no en la mejor ronda** |
| Peor caso, p99 ≤ +5 % | sin repeticiones, 100 000 | −8,5 % → −7,0 % | +3,2 % → −2,8 % | **cumple** |
| (complementaria) W1 p50 | `w1-index-stale`, 20 000 | −100,0 % → −100,0 % | −100,0 % → −100,0 % | cumple |
| (complementaria) W1 p99 | `w1-index-stale`, 20 000 | −11,9 % → −6,7 % | −13,7 % → −5,8 % | cumple |
| Escrituras: sin cambio medible en U1 | — | — | — | no medido aquí: la caché no escribe en el motor, lo comprueba E6-03 (bytes y `commit_seq` idénticos) |

Los valores «antes» son los de E6-04 (corrida del 2026-11-09); los «ahora», los de `etapa6-medicion-salida.txt`, corrida de E6-04b. Son corridas distintas de la misma máquina: la diferencia entre ellas incluye la dispersión entre corridas (§7).

Valores absolutos de E6-04 (**antes** de E6-04b; los actuales están en `etapa6-medicion-salida.txt`):

| Carga | Modo | p50 (rango de rondas) | p99 (rango de rondas) |
|---|---|---|---|
| `w1-index-ready` 20 000 | apagada | 1,212 – 1,390 | 4,668 – 5,534 |
| `w1-index-ready` 20 000 | encendida | 0,028 – 0,032 | 4,760 – 5,716 |
| sin repeticiones 20 000 | apagada | 0,857 – 0,913 | 1,018 – 1,809 |
| sin repeticiones 20 000 | encendida | 0,936 – 0,974 | 1,231 – 1,705 |
| sin repeticiones 100 000 | apagada | 6,490 – 6,922 | 8,591 – 9,673 |
| sin repeticiones 100 000 | encendida | 6,814 – 7,077 | 7,864 – 10,215 |

### Lectura honesta (de E6-04, antes de E6-04b)

- **p50 de W1: la mejora es la esperada y no es frágil.** Una lectura servida de caché cuesta ≈ 0,03 ms frente a ≈ 1,3 ms. El 75 % de las lecturas se sirven; el p50 cae en el bloque servido.
- **p99 de W1: no se puede afirmar que se cumpla.** El p99 de W1 lo fija la lectura que **no** se sirve (la primera tras cada escritura, el 25 % de las lecturas), que con la caché encendida hace lo mismo que sin ella **más** el coste de guardar el resultado. Las rondas se solapan: apagada 4,67–5,53 ms, encendida 4,76–5,72 ms. Con el método de E6-00 (mejor ronda) da +2,0 %, dentro; con la mediana da +7,8 %, fuera. La diferencia entre las dos lecturas es del orden de la dispersión entre rondas, así que la medición no distingue «dentro del 5 %» de «fuera»; más rondas la acotarían, pero no es un ajuste que corresponda hacer aquí.
- **Peor caso: sobrecoste fijo por lectura de ≈ 0,07 ms** (p50 +9,2 % a 20 000 unidades). Es el coste de calcular la huella, consultar la caché y guardar las filas, que se ve como un porcentaje alto cuando la lectura base cuesta ≈ 0,9 ms (20 000 unidades) y se diluye cuando cuesta ≈ 7 ms (100 000 unidades: p50 +3,3 % a +5,0 %, p99 dentro). El p99 es ruidoso entre rondas (a 100 000, de 7,86 a 10,22 ms con la caché encendida), pero a 20 000 el sobrecoste de p50 es consistente (apagada 0,857–0,913 ms frente a encendida 0,936–0,974 ms: los rangos no se solapan). El p99 sí se solapa (1,02–1,81 frente a 1,23–1,71 ms): el +20,9 % de la mejor ronda depende de una ronda apagada excepcionalmente baja (1,018 ms), pero la mediana también incumple (+11,7 %), de modo que las dos formas de agregar dan «no cumple».
- **Lo que no dice esta medición.** No hay W1 a 100 000 unidades, no se varía la proporción de repeticiones ni el tamaño de la caché, y la carga sin repeticiones usa un único patrón (todas las lecturas sustituyen la entrada anterior). Un patrón sin sustitución (más formas de consulta distintas que el tipo único) podría costar menos o más; no se ha medido.

### Consecuencia para la decisión 2 de la acta (E6-04, antes de E6-04b)

El criterio de §6 es conjunto («mejora neta»: p50 y p99 de W1 **y** peor caso). Con estos números **no se cumple** de forma que se pueda afirmar sin discusión: la caché sigue **desactivada por defecto** (acta §8.2) y la decisión posterior —aceptar el resultado con la caché desactivada por defecto, optimizar el camino de fallo (por ejemplo, no guardar hasta una segunda lectura, que exigiría un contador propio sin leer el observador, G3) o repetir con otra carga— es del orquestador. E6-04 se detiene aquí: no se ha tocado el código de la caché ni los umbrales.

## 3b. E6-04b: qué se cambió en el camino de fallo

Medido con un perfil temporal (ya retirado) en el planificador: el trabajo propio de la caché en un fallo es de ≈ 2–3 µs por lectura (huella ≈ 1,3 µs, `lookup` ≈ 0,5–0,7 µs, decisión de guardar ≈ 0,2–0,9 µs) frente a ≈ 850 µs de la lectura. El coste que más pesaba era el de **guardar** el resultado de cada consulta que no se repite (copiar las filas y expulsar la entrada anterior), y la dispersión entre rondas es mucho mayor que el resto. Cambios:

1. **Admisión** (`ResultCache::worth_storing`, `cache.rs`): una entrada cabe por forma de consulta. Un recién llegado con otros literales sustituye a una entrada **que no ha servido nada** solo en su **segunda aparición** (la primera se rechaza y se recuerda en memoria, una consulta por forma). Se guarda de inmediato la primera consulta de una forma, la misma consulta que la entrada, un recién llegado frente a una entrada que ya sirvió, y uno frente a una entrada cuyas dependencias ya no coinciden. No lee el observador (G3), no deja registro y no cambia lo que se sirve: solo decide si se guarda (G1–G7 intactas). El estado rechazado se borra con `clear`, `set_key` y al apagar la caché. Pruebas: 4 unitarias en `cache.rs` y `tests/cache_admission.rs`.
2. **Huella memorizada por forma** (`fingerprint_of`): la huella depende solo de la forma (qué condiciones están puestas y la dirección), así que se calcula una vez por forma y clave y se reutiliza, en vez de construir el texto de la forma y hacer BLAKE3 en cada lectura. Se descarta al cambiar la clave. Prueba: `the_remembered_fingerprint_of_a_shape_is_the_one_computed_from_its_text` compara cada una de las 8 formas con la huella calculada del texto (en rojo si se altera la numeración de las ranuras).
3. **Control de la carga sin repeticiones** (`index_bench.rs`): antes exigía 0 lecturas servidas. Con la admisión, la entrada de la primera consulta se conserva y sirve la lectura en que su tipo vuelve (una de cada 100: 36 de 3 600, 13 de 1 350). El control ahora exige que no haya más servidas que esas recurrencias (`⌈lecturas/100⌉ + 1`). No es un umbral: protege que la carga siga siendo el peor caso.

## 4. Centinelas (I-37)

`crates/kerno-units/tests/cache_sentinels.rs`, 4 pruebas (2 × dos motores: `Emulated<LogEngine>` y `KernoDb`). `cache.rs` (E6-01) ya planta los centinelas de `sentinels.rs` y recorre el almacén; estas pruebas cubren lo que ese archivo deja abierto. Conteo de pruebas de `kerno-units`: 305 → 309.

| Prueba | Qué afirma |
|---|---|
| `no_literal_in_anything_the_cache_path_reports` | Con la caché encendida, **en cada nivel de observación** (`Off`, `Aggregate`, `Fingerprint`, `Diagnostic`), pasa por el planificador real consultas de unidades (tipo, versión de esquema, ambos) y de relaciones, leídas dos veces (fallo y acierto), una entrada forjada que no pasa las comprobaciones (discrepancia), el interruptor por consulta, una escritura con centinelas que invalida por secuencia, y los **errores** del almacén (entrada rechazada de cada clase, activar una huella desconocida, entrada mayor que `max_bytes`, límite inválido, caché apagada). Después repite todo sobre una unidad con la cabeza dañada, de modo que la lectura termina en error. Concatena todo lo que se informa —`source`, `path` y `fallback` de cada lectura, la explicación del plan, los registros y contadores de la caché, los del observador, cada error con `Display` y `Debug`— y no encuentra ningún centinela (tipo, carga, metadatos, tipo de relación, fuente y origen de procedencia, versión de esquema) en texto, lista decimal ni hexadecimal. **Control:** las respuestas entregadas al llamante sí contienen los centinelas, así que la búsqueda podía encontrarlos. Además, toda huella informada es la de la misma forma con otros literales (una huella que mezclara un literal se delataría). |
| `every_g7_transition_leaves_its_record` | Un caso por **cada hoja** de `Transition` (27: cada causa de fallo y de miss, cada dependencia, cada motivo de borrado, cada interruptor), cada uno sobre un almacén nuevo. Exige que el caso deje su registro con `optimization_version` y `planner_version`, con la huella de la consulta (o ninguna en el interruptor global y en `clear`), con la `commit_seq` de la entrada cuando el registro trata de una, y sin literales en el registro, los contadores ni los errores del almacén. `label` no tiene comodín: una variante nueva no compila hasta que se nombra allí y en `ALL_LABELS`, y la prueba exige que el recorrido tenga exactamente esos casos. |

### Mutaciones probadas

Cada mutación se aplicó a `crates/kerno-units/src/cache.rs`, se corrió `cargo test -p kerno-units --all-features --release --test cache_sentinels` y se restauró el archivo (`git checkout`).

| ID | Mutación | Resultado |
|---|---|---|
| S-M1 | **Copiar un literal en el registro de la optimización:** tras registrar `Used`, escribir el valor del filtro `schema_version` en `commit_seq` del registro (`r.commit_seq = Some(CommitSeq(u64::from(v)))`) | **Mueren** `emulated_log::no_literal_in_anything_the_cache_path_reports` y `kerno_db::no_literal_in_anything_the_cache_path_reports`: `schema version sentinel "4071337791" leaked in the output at level Off`. Las dos pruebas de transiciones siguen en verde (el filtro del caso no lleva versión): la fuga la ve el centinela, no el recorrido |
| S-M2 | No dejar el registro `Used` en un acierto (`st.record(..., Transition::Used, ...)` sustituido por una expresión sin efecto) | **Mueren las cuatro**: `Used: no record in […]` en las de transiciones y `level Off: no Used record in {…}` en las de literales |

Todas las pruebas vuelven a verde al restaurar el archivo (`4 passed`).

## 5. Estado de los criterios del acta (§4)

| Criterio | Evidencia en E6-04 |
|---|---|
| Mejora neta medida frente al modo desactivado, con umbrales fijados antes | medida y publicada (§3); p50 de W1 cumple; los p99 **no** se cumplen de forma estable: **no concluido a favor**, decisión del orquestador |
| Trazabilidad sin literales (G7, I-37) | las dos pruebas de §4 y las mutaciones S-M1, S-M2 |

La actualización de la tabla del acta (§4) corresponde a E6-05, como con los paquetes anteriores.

## 6. Lo que E6-04 no cubre

- W1 a 100 000 unidades, otras proporciones de repetición, la influencia de `max_entries`/`max_bytes` en el peor caso y el patrón de peor caso con más formas de consulta distintas.
- U1 (escrituras): no se midió; el argumento es estructural (la caché no tiene manejador del motor) y E6-03 lo comprueba por bytes y `commit_seq`.
- Concurrencia entre lecturas que comparten la caché: la medición es de un solo hilo.
- Una causa del sobrecoste del peor caso medida por separado (huella, `lookup`, guardado): se atribuye por razonamiento, no por perfil.

## 7. Verificación de E6-04b y límites

- Comando: `cargo run --release -p kerno-units --example index_bench -- --cache` (5 rondas, salida en `etapa6-medicion-salida.txt`, `exit=0`). Máquina de 4 CPU, carga ≈ 1 durante la corrida.
- **La medición de 5 rondas es ruidosa en el borde del umbral.** Con la misma lógica de producción, otra corrida de 5 rondas dio, a 20 000 sin repeticiones, p99 mediana +12,1 % y +9,2 % de p50 antes de memorizar la huella; una de 15 rondas (solo esa carga, edición temporal del banco, ya revertida) dio p99 mediana +6,9 % y p50 +3,4 %. Una comparación entrelazada consulta a consulta (caché apagada/encendida en bloques de 100) midió entre +1 % y +3 % de media (9–25 µs sobre ≈ 850 µs); el control apagado-contra-apagado dio −0,7 %. Es decir: **el sobrecoste real del peor caso a 20 000 unidades es del orden de 1–3 %, por debajo del 5 %, pero la lectura de p99 de 5 rondas puede salir a uno u otro lado**. Por eso el veredicto de este documento es el de la mediana, que es el criterio pedido, y se publica también el de la mejor ronda, que sigue fuera a 20 000 (+9,7 %).
- No se explica por el trabajo propio de la caché (≈ 3 µs) el resto del sobrecoste observado (≈ 10–25 µs): la sección del planificador que no toca la caché también salió ≈ 15–25 µs más lenta con la caché encendida, incluso sin guardar nada. Se atribuye a efectos de caché de CPU y del asignador, **sin perfil que lo demuestre**.
- Sigue sin medirse W1 a 100 000 unidades ni concurrencia (ver §6).
- Verificación: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --all-features -- -D warnings` y `cargo test -p kerno-units --all-features --release` (resultado en el informe del paquete). Los umbrales y las pruebas `tests/cache*.rs` existentes no se modificaron.
