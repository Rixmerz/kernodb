# ADR-0014 — Representación física de relaciones

| Campo | Valor |
|---|---|
| Estado | Aceptado (opción 2; evidencia de E4-02) |
| Fecha | 2026-10-28 (propuesta); 2026-11-05 (aceptado con la evidencia de E4-02) |
| Decisores | Propietario del proyecto (aprobó la semántica del criterio 4 en E4-00); ratificación de esta decisión en el acta de E4-02 |
| Relacionado con | [especificación v0.2.0](../spec/especificacion-arquitectonica-v0.2.0.md) §6.4, §10, §12; SAD §9 (tabla de estructuras, fila 7); ADR-0001, ADR-0004, ADR-0007, ADR-0010; invariantes I-21, I-39 |

## Contexto

La Etapa 4 introduce relaciones tipadas (`Edge`) entre unidades: `edge_id`, `from_unit_id`, `to_unit_id`, `relation_type`, peso y validez temporal opcionales, procedencia y atributos. La especificación pide no asumir que todas son dirigidas ni que tienen peso, y separar relaciones declaradas de inferidas. El SAD (§1 y §20, «Fuera de la primera tanda») excluye un lenguaje de grafos y reserva un motor de grafos especializado para cuando un workload lo justifique.

El motor físico es un LSM nivelado (ADR-0001) con lotes atómicos, aislamiento por instantánea y recorridos por rango. La pregunta es cómo codificar relaciones sobre ese motor.

## Opciones consideradas

1. **Relaciones como unidades** con un tipo reservado, sin índice de adyacencia. Mínima superficie; recorrer vecinos exige un filtro sobre todas las relaciones.
2. **Registro de relación + índices de adyacencia transaccionales** (propuesta). Un registro por `edge_id` y dos entradas de índice con claves ordenadas `out/{from}/{type}/{edge_id}` e `in/{to}/{type}/{edge_id}`, escritos en el mismo lote que el registro. Vecinos por rango; coste: tres escrituras por relación.
3. **Listas de adyacencia por unidad** (un valor con todos los vecinos). Lectura de vecinos en una sola clave; reescritura completa de la lista en cada cambio y conflictos de escritura en nodos con muchas relaciones.
4. **Motor de grafos dedicado**. Fuera de alcance hasta que un benchmark lo justifique (SAD, spec §5).

## Criterios de decisión

Fijados el 2026-11-05, **antes** de la medición completa de «Evidencia». Los cinco criterios son los de la propuesta; E4-02 los vuelve medibles con un umbral o una prueba que decide cada uno. El banco se ejecutó antes en modo `--quick` solo para depurarlo; sus resultados no se usaron para elegir los umbrales. Una opción que incumple un criterio de corrección (1, 2, 4, 5) no se acepta, sea cual sea su rendimiento; los criterios de coste (3) se juzgan entre opciones.

1. **Atomicidad.** El registro de una relación y sus entradas de índice se confirman en la misma frontera que el resto del cambio (I-21). Se cumple si, en un barrido de caídas antes de cada operación mutante (corte de energía y muerte del proceso, 4 semillas, `Emulated<LogEngine>` y `KernoDb`), el almacén recuperado es siempre el de un prefijo entero de `ChangeSet` (byte a byte frente a un oráculo sin fallos) y `verify_units` no da hallazgos: **cero** estados con el registro sin sus entradas o al revés.
2. **Equivalencia.** Recorrer vecinos por índice devuelve exactamente lo que devuelve la ruta de referencia (filtro sobre todos los registros, sin índice) sobre la misma instantánea (I-32). Se cumple con **cero** diferencias, para toda unidad, dirección y tipo, tras cada paso de cargas aleatorias con semilla (altas, bajas y abortos), con más relaciones por unidad que una página de lectura (256) y en las instantáneas antiguas; y en el banco, 20 nodos × 1 y 2 saltos entre las dos opciones.
3. **Coste.** Sobre dos cargas sintéticas con la forma de los workloads del roadmap, ambas con 5 000 unidades: `livespec` (símbolos y especificaciones; grado de salida 5; la mitad de las relaciones apuntan al 5 % de las unidades) y `memory` (memoria de agentes; cadena más enlaces a elementos recientes; grado 3, sin concentradores):
   - (a) Escritura por relación: claves, bytes de WAL y bytes volcados de la opción 2 **≤ 1,5×** los de la opción 1.
   - (b) Lectura: p99 de recorrer 1 y 2 saltos de la opción 2 **≤** el de la opción 1 en las dos cargas, y p99 de 1 salto de la opción 2 **≤ 1 ms**.
   - (c) Efecto sobre lo ya aprobado: con relaciones activas, U2 y U6 se re-miden **con los umbrales sin cambiar** (U2: p99 de `history` ≤ 1,5× el de la referencia; U6: p99 y rendimiento de escritura durante la poda no peores que antes, con la misma carga) y no salen peor que el rango ya documentado en el informe de la Etapa 3 (U2 0,95×–1,67×; U6 p99 de escritura ≤ 1,59× y rendimiento ≥ 0,76×). Un incumplimiento previo (K-E3-8, K-E3-9) no se atribuye a las relaciones si queda dentro de ese rango.
4. **Archivar o borrar con relaciones.** Semántica explícita, aprobada por el propietario en E4-00: **rechazo**. Se cumple si archivar y borrar una unidad con una relación viva, por cualquiera de sus dos extremos, falla con un error que nombra la unidad y una relación, sin efecto sobre el almacén y sin romper el `ChangeSet`, en `Emulated<LogEngine>` y `KernoDb`; y si borrar antes las relaciones (incluso en el mismo `ChangeSet`) permite archivar o borrar.
5. **Procedencia (I-39).** Toda relación lleva un campo de procedencia obligatorio (`declared`, `imported`, `algorithmic`, `inferred`) que no se puede omitir (sin valor por defecto), con origen para todas salvo `declared` y confianza solo para `inferred`. Se cumple si el escritor y el decodificador rechazan toda combinación incoherente, y si una relación inferida no se devuelve nunca como declarada por ninguna de las dos rutas de lectura, ni siquiera con los mismos extremos y tipo que una declarada.

## Evidencia

Prototipo y mediciones de E4-02 (2026-11-05), en `kerno-units` sobre `kerno-storage` (`KernoDb`, opciones por defecto: `Sync`, caché 64 MiB, búfer de escritura 8 MiB, sistema de archivos real), un solo equipo, sin otras cargas deliberadas (carga media de 1,0 al empezar). Código: relaciones en `crates/kerno-units/src/db.rs`, `codec.rs` y `reference.rs`; bancos `examples/relation_bench.rs` (opción 1 frente a opción 2) y `examples/unit_bench.rs --relations` (U2 y U6).

**Qué se construyó.** Opción 2: `create_relation` escribe `REL` y sus entradas `REL_OUT` y `REL_IN` en el mismo `ChangeSet`; `delete_relation` las borra juntas; `relations_of` lee la adyacencia por rango y comprueba cada entrada contra su `REL`; la ruta de referencia (`reference_relations`, `reference_relations_of`) recorre todos los `REL` y nunca una entrada de índice. Opción 1: una relación es una unidad de tipo reservado con la carga `from ‖ to ‖ procedencia ‖ tipo`; los vecinos se obtienen filtrando todas (`reference_by_type`); en 2 saltos lee todas una vez y filtra la copia dos veces, que es su mejor caso. Formato: [units-v1.md](../file-format/units-v1.md) §3.7–§3.8 (tags `0x40..=0x42`, regla UNIT-36).

**Criterio 3 (a) y (b), `relation_bench` completo** (5 000 unidades; `livespec` 25 000 relaciones, `memory` 15 000; consultas: 2 000 por salto y carga en la opción 2, 200 en la opción 1, tras un 10 % de calentamiento):

| | Opción 1 (unidad por relación) | Opción 2 (registro + 2 índices) | Opción 2 / opción 1 |
|---|---|---|---|
| Claves escritas por relación | 4,00 | 3,00 | 0,75× |
| WAL por relación | 351,2 B | 198,2 B | 0,56× |
| Volcado a tablas por relación | 382,9 B | 221,3 B | 0,58× |
| `livespec` 1 salto: p50 / p99 | 87,10 / 130,64 ms | 0,0074 / 0,0244 ms | ≈ 1/5 000 (p99) |
| `livespec` 2 saltos: p50 / p99 | 86,34 / 123,00 ms | 0,0416 / 0,1024 ms | ≈ 1/1 200 (p99) |
| `memory` 1 salto: p50 / p99 | 55,90 / 73,96 ms | 0,0049 / 0,0119 ms | ≈ 1/6 200 (p99) |
| `memory` 2 saltos: p50 / p99 | 57,31 / 89,77 ms | 0,0162 / 0,0494 ms | ≈ 1/1 800 (p99) |

La opción 1 escribe *más* porque cada relación hereda la maquinaria de la unidad (cabeza, `REV_META`, `REV_PAYLOAD` y miembro del `ChangeSet`); la opción 2 escribe tres claves pequeñas. Compactación: 0 bytes por relación en ambas a este tamaño (todo cabe en el volcado). Los dos lados devolvieron los mismos vecinos en 20 nodos × 1 y 2 saltos en cada carga.

**Criterios 1, 2, 4 y 5, pruebas** (todas pasan en `Emulated<LogEngine>` y `KernoDb`; ejecución: `cargo test -p kerno-units --test relations --test crash`):

- Criterio 1: `crash::{emulated_log,kerno}::relations_every_point_{power_loss,process_kill}`, 49 puntos por semilla × 4 semillas; almacén igual al oráculo en todos, `verify_units` limpio, y la base recuperada acepta una relación y su borrado. La prueba se vio fallar al compararla con el prefijo equivocado. Filas R1–R5 de la [matriz de fallos](../architecture/failure-matrix.md) §8.
- Criterio 2: `relations::differential_seeded` (4 semillas × 40 pasos, 12 unidades, 3 tipos donde uno es prefijo de otro, con abortos), `differential_across_scan_pages` (600 relaciones sobre 3 unidades) y la comparación del banco: 0 diferencias.
- Criterio 4: `relations::archive_and_delete_are_rejected_with_live_relations`: rechazo por ambos extremos, error `UnitHasRelations` que nombra la unidad y la relación, el `ChangeSet` sigue siendo utilizable, y archivar o borrar tras borrar las relaciones en el mismo `ChangeSet` funciona. La prueba se vio fallar al invertir su aserción.
- Criterio 5: `relations::provenance_is_stored_and_checked` y `declared_is_never_confused_with_inferred` (también a nivel de bytes: cambiar el tipo de procedencia de un registro inferido a declarado o importado lo hace indecodificable).

**Criterio 3 (c): U2 y U6 re-medidos, umbrales sin cambiar** (`unit_bench` completo: mismos tamaños y método que el informe de la Etapa 3; `--relations` añade 3 relaciones salientes por unidad antes de medir y, en U6, una relación por cada `update` de los escritores, en el mismo `ChangeSet`). Escala completa, **sin reducir**; U2 en 3 pasadas por modo, U6 en **1** pasada por modo (≈ 5 min cada una; no se repitió).

| U2: p99 `history` / referencia (umbral ≤ 1,5×) | Pasada 1 | Pasada 2 | Pasada 3 |
|---|---|---|---|
| Sin relaciones | 0,0780 / 0,0517 ms = **1,51×** (incumple por 0,01) | 0,0771 / 0,0586 = 1,31× | 0,0663 / 0,0447 = 1,48× |
| Con relaciones | 0,0781 / 0,0626 = 1,25× | 0,0755 / 0,0518 = 1,46× | 0,0800 / 0,0543 = 1,47× |

U2 queda en el rango del informe de la Etapa 3 (0,95×–1,67×), con la misma oscilación alrededor de 1,5× (K-E3-8); `history_page` 0,70×–1,00× de la referencia, informativo. Las relaciones no están en el camino de `history`.

| U6 (retención `KeepLast(1)` con 2 escritores y 1 lector) | p99 escritura durante / peor «antes» | Rendimiento escritura durante / menor «antes» | p99 lectura durante / peor «antes» | Retención | Veredicto |
|---|---|---|---|---|---|
| Presupuesto por defecto, sin relaciones | 2,802 / 2,908 ms = 0,96× | 2 002,5 / 2 283,0 = 0,88× | 0,0225 / 0,0081 = 2,77× | 16 888 rev/s | incumple |
| Presupuesto por defecto, con relaciones | 2,399 / 2,237 ms = 1,07× | 2 316,8 / 2 405,2 = 0,96× | 0,0097 / 0,0074 = 1,30× | 21 633 rev/s | incumple |
| 25 unidades, sin relaciones | 2,769 / 2,567 ms = 1,08× | 2 188,6 / 2 369,9 = 0,92× | 0,0317 / 0,0124 = 2,55× | 12 946 rev/s | incumple |
| 25 unidades, con relaciones | 2,640 / 2,182 ms = 1,21× | 2 210,2 / 2 362,7 = 0,94× | 0,0260 / 0,0244 = 1,07× | 12 796 rev/s | incumple |

**U6 sigue incumpliendo (K-E3-9), con y sin relaciones, igual que al cierre de la Etapa 3.** Con relaciones activas el p99 de escritura (1,07× y 1,21×) y el rendimiento (0,96× y 0,94×) quedan dentro del rango ya documentado (p99 ≤ 1,59×, rendimiento ≥ 0,76×); no se atribuye a las relaciones ningún empeoramiento, pero con una pasada por modo y ventanas «antes» que en el informe de la Etapa 3 varían 2–3× entre sí, la diferencia entre «con» y «sin» (0,96× frente a 1,07×, etc.) está dentro del ruido y no demuestra que las relaciones sean gratis en U6. Ningún umbral se ha bajado ni se declara cumplido.

**Límites de esta evidencia.** Cargas sintéticas de 5 000 unidades y 15 000–25 000 relaciones, no los workloads reales de LiveSpec ni de memoria de agentes (no existen aún); el mayor grado de entrada es de unos 50 por concentrador, muy lejos de las 10⁵ relaciones de «Condiciones para reabrir», que **no** se han medido. Las latencias de la opción 1 dependen de que se lea todo el conjunto de relaciones; una opción 1 con un índice añadido es, en la práctica, la opción 2. La opción 3 (listas de adyacencia por unidad) y la 4 no se prototiparon: se descartan por el argumento de la propuesta (reescritura completa de la lista en cada cambio y conflictos en nodos con muchas relaciones), que esta evidencia no contradice ni confirma.

## Decisión

**Opción 2: registro de relación más índices de adyacencia transaccionales**, aceptada el 2026-11-05 con la evidencia de E4-02. Cumple los cinco criterios; la opción 1 cumple 1, 2, 4 y 5 por construcción pero pierde en el 3 (b) por cuatro órdenes de magnitud y escribe más por relación (3 (a)). Lo que se decide:

- Una relación es dirigida, con `edge_id`, `from`, `to`, `relation_type` (1..=64 B), `created_seq` y procedencia obligatoria (`declared`, `imported`, `algorithmic`, `inferred`; origen salvo en `declared`; confianza en milésimas solo en `inferred`). El actor, el origen y el motivo son los del `ChangeSet` que la creó.
- El registro y las dos entradas de índice se escriben y borran siempre en el mismo `ChangeSet`.
- Archivar o borrar una unidad con relaciones vivas, por cualquiera de sus extremos, se **rechaza** (`UnitError::UnitHasRelations`); crear una relación exige dos unidades `Active` en el `ChangeSet`. Es relajable a cascada o a relación colgante marcada sin cambiar el formato (decisión del propietario, E4-00).
- Una relación **no tiene revisiones propias**: crearla o borrarla es un cambio de un `ChangeSet`, y ese `ChangeSet` es la única huella (resuelve la duda de ADR-0004 para esta etapa). Peso y validez temporal opcionales **no** se incluyen en E4-02; añadirlos es un campo nuevo del registro y requiere decidir la interacción con el historial.
- Las relaciones cuentan contra el límite de operaciones de un `ChangeSet` y un `ChangeSet` que solo crea o borra relaciones es un commit (no está vacío).

## Consecuencias

- Las claves de adyacencia pasan a formar parte del esquema lógico de claves; su codificación se versiona con el resto del formato lógico (ADR-0003, ADR-0008). E4-02 las define en [units-v1.md](../file-format/units-v1.md) §2, §3.7 y §3.8 **sin subir `layer_version`** (sigue en 1): son tags nuevos (`0x40..=0x42`) que una capa anterior rechaza como `IncompatibleLayer` (UNIT-2) al verificar, pero una capa anterior que abriera el almacén sin recorrerlo no conocería el rechazo de archivar con relaciones. Mientras no haya almacenes v1 en uso fuera del desarrollo es aceptable; antes de congelar el formato de la capa, el propietario debe decidir si basta o si hay que subir `layer_version` a 2.
- Las relaciones con validez temporal interactúan con el historial (ADR-0004): por ahora la relación no tiene revisiones propias ni validez temporal (véase «Decisión»).
- Una relación hacia una unidad suprimida queda sujeta a ADR-0009. Con el rechazo de E4-02 no puede existir una relación hacia una unidad archivada o borrada, por lo que la supresión física no tiene relaciones colgantes que tratar mientras se mantenga ese rechazo.
- El borrado de una unidad exige dos lecturas de rango de una entrada (una por índice) dentro de la `Txn`; el coste de las relaciones en la ruta de escritura de `ChangeSet` sin relaciones no se ha medido por separado de U1/U2/U6.
- Los índices de adyacencia son **primarios** (se actualizan en el mismo `ChangeSet`, ADR-0007), no secundarios con estado de frescura: E4-03 no necesita watermark para ellos. `UnitSnapshot::relations_of` comprueba cada entrada contra su `REL`.

## Condiciones para reabrir

Workloads con recorridos de más de dos saltos dominantes, o nodos con más de 10⁵ relaciones donde el coste de la opción 2 sea inaceptable.

## Seguimiento

- ~~Prototipo y benchmark al inicio de la Etapa 4.~~ Hecho en E4-02 («Evidencia»).
- ~~Prueba de propiedades: índices de adyacencia coherentes con los registros tras fallos (matriz de fallos).~~ Hecho: [matriz de fallos](../architecture/failure-matrix.md) §8, filas R1–R5.
- Repetir U6 (varias pasadas, con y sin relaciones) cuando se aborde K-E3-9; medir un nodo con 10⁵ relaciones antes de dar por descartadas las «Condiciones para reabrir».

## Nota de E4-00 (2026-11-04)

Estaba *Propuesto* en esa fecha. La decisión final sale del prototipo de E4-02, que la aceptó el 2026-11-05 (opción 2). Semántica aprobada por el propietario para unidades archivadas o borradas con relaciones vivas: **rechazo** de la operación (relajable más adelante a cascada o relación colgante marcada).
