# Acta de entrada: adelanto de la Etapa 9 (endurecimiento) en paralelo con la Etapa 7

| Campo | Valor |
|---|---|
| Fecha | 2026-11-11 |
| Estado | **Aprobada por delegación del propietario el 2026-11-11, revocable** |
| Acto | Propuesta de alcance y de los paquetes E9A-00 a E9A-06: qué parte de la Etapa 9 se adelanta antes de la Etapa 8 y qué parte corre en paralelo con la Etapa 7 |
| Documentos | [Roadmap (pendiente 1 y Etapa 9)](../roadmap/README.md) · [acta de la Etapa 7](etapa7-acta.md) · [acta de la Etapa 6](etapa6-acta.md) · [SAD §11, §16 (Fases 4 y 6) y §18](SAD.md) · [invariantes](invariants.md) · [matriz de fallos](failure-matrix.md) · [threat model](../security/threat-model.md) · [ADR-0005](../adr/0005-identificadores-publicos.md), [ADR-0006](../adr/0006-cifrado-y-gestion-de-claves.md), [ADR-0009](../adr/0009-supresion-retencion-y-backups.md), [ADR-0012](../adr/0012-metodologia-de-benchmarks.md), [ADR-0013](../adr/0013-formato-v1-y-protocolo-del-motor.md), [ADR-0015](../adr/0015-privacidad-de-la-observabilidad.md) · [cierre de la Etapa 3 (K-E3-7)](etapa3-cierre.md) · [diseño de `kerno-storage`](../../crates/kerno-storage/DESIGN.md) |

## 0. Estado

Esta acta está **aprobada por delegación** (§10), revocable por el propietario. Resuelve la decisión pendiente 1 del roadmap («Orden de seguridad frente a IA») en el sentido propuesto allí: lo que afecta a datos derivados se adelanta antes de la Etapa 8; el resto queda en la Etapa 9.

Los paquetes **no son secuenciales**: §3.1 indica cuáles corren a la vez. Ninguno escribe en `crates/kerno-units` ni en `crates/kerno-testkit`, que son las superficies de la Etapa 7 (commits `34004f0` y `60de6d8`). Ningún paquete arranca la Etapa 8.

## 1. Condición de inicio

| Condición | Estado |
|---|---|
| Pendiente 1 del roadmap | Resuelta por delegación el 2026-11-11 (§10) |
| Etapa 7 en curso | E7-00 y E7-01 hechos; E7-02 es el siguiente. **No bloquea**: los paquetes E9A no tocan sus archivos |
| ADR-0009 (supresión, retención y backups) | **Aceptado el 2026-11-12, opción 3, en los términos de la ADR** (E9A-00b, hecho). Ya no bloquea la entrada en la Etapa 8. No autoriza ninguna API de supresión, que no está en esta acta |
| ADR-0015 (privacidad de la observabilidad) | Aceptado el 2026-11-06. Es la base de E9A-02 |
| ADR-0013 (formato v1) | **Resuelto el 2026-11-11** (§7, punto 3): aceptado y formato v1 congelado el 2026-10-31. No bloquea ningún E9A; las migraciones de la Etapa 9 siguen pendientes como trabajo propio |
| Etapa 8 | Arranca por decisión del propietario el 2026-11-12 ([cierre de la Etapa 7 §8.1](etapa7-cierre.md)). Su condición sobre ADR-0009 se cumple desde el 2026-11-12. ADR-0018 quedó aceptado el 2026-11-12 con la opción 2, en el acta de la Etapa 8. Propuesta de esta acta, revocable: que E9A-02 esté cerrado antes de que la Etapa 8 escriba ningún derivado |

## 2. Alcance

### 2.1 Objetivos

1. Fijar los criterios de ADR-0009 **antes de medir** y medir qué sobrevive a un borrado en `kerno-storage` (E9A-00a, E9A-01). Aceptar o rechazar ADR-0009 es decisión del propietario (E9A-00b).
2. Verificar I-29 en `kerno-storage`: errores, verificación y métricas sin contenido de documentos (E9A-02). Hoy no tiene prueba automatizada.
3. Endurecer los lectores de `kerno-storage` frente a entradas hostiles: propiedades de apertura (E9A-03), fuzzing con cobertura si el propietario aprueba la dependencia (E9A-04) y estrés del ciclo de vida (E9A-05).

### 2.2 Punto de partida (lo que se leyó)

- **Entradas que `kerno-storage` parsea.** Los decodificadores puros están en `crates/kerno-storage/src/format/`: cabecera común de 96 bytes, registros del WAL y su escáner, bloques de datos, *footer*, filtro, propiedades, fragmentos de rango y registros del MANIFEST en TLV. Los conectan `wal.rs`, `sst.rs`, `block.rs` y `manifest.rs` a través del `Vfs`. El API pública recibe claves y valores, que `db.rs` limita antes de escribir (`check_put`, `check_delete`, `check_range`).
- **Qué emite.** Ningún archivo de `crates/kerno-storage/src` llama a `log`, `tracing`, `println!`, `eprintln!`, `stdout` ni `stderr`. La salida observable son tres cosas: el enum `Error` de `kerno-core` (siete variantes, cuatro de ellas con `String`), los `Issue` de `verify()` (su documentación dice «never key or value bytes») y las métricas de `metrics.rs`, que son contadores e histogramas. `format/mod.rs` declara que los errores de decodificación nunca llevan bytes de clave ni de valor (T-08). Los mensajes con datos interpolados que se leyeron contienen nombres de archivo, reglas y números (`sst.rs:332`, `db.rs:1446`).
- **Pruebas de centinelas.** No hay ninguna en `kerno-storage` ni en `kerno-testkit` (búsqueda de `sentinel` y `centinela`: sin resultados). Según la regla de [invariantes §0](invariants.md), I-29 está **pendiente** para `kerno-storage`.
- **Supresión.** Existen `delete` y `delete_range` en la transacción (`txn.rs:150` y `txn.rs:161`), `compact_range` (`db.rs:2011`), el borrado diferido de tablas obsoletas por el hilo `kerno-delete` (`deleter.rs`) y el borrado de WAL tras el `FLUSH`. Con `wal_preallocate` (por defecto `false`, `config.rs:543`) un WAL ya volcado se sobrescribe con ceros y se reutiliza. [DESIGN.md](../../crates/kerno-storage/DESIGN.md) dice que `delete_range` y `compact_range` son **mecanismo**, sin garantía de borrado físico, y que «el diseño de supresión con garantías (ADR-0009) queda para la Fase 4». El [threat model §6.5](../security/threat-model.md) deja T-09 abierto. No hay cifrado: el formato reserva AEAD y lo rechaza como `Incompatible`.
- **Pruebas de entradas hostiles que ya existen.** En `tests/struct_vectors.rs`, `decoders_never_panic_on_random_bytes` genera entradas aleatorias de semilla fija para 14 tipos de decodificador: 100 000 por tipo por defecto (`KERNO_FUZZ_INPUTS`) y exige que ninguna se acepte. `spec_decoder_agrees` compara 5 000 por tipo (`KERNO_AGREE_INPUTS`) con un decodificador independiente. Hay además `crafted.rs` (archivos fabricados con CRC válido), `damage.rs`, `verify_detects_every_bit_flip` y `tests/model.rs` (proptest sobre el modelo). **No hay fuzzing con cobertura**: no existe directorio `fuzz/` ni dependencia `libfuzzer`, y el [threat model §6.2](../security/threat-model.md) lo sitúa en la Fase 6, pendiente.
- **Estrés que ya existe.** `tests/group_commit.rs::stress_handoff_with_random_delays`: cuatro escritores, con 1 500 commits cada uno en un caso y 1 000 en el otro, y retrasos aleatorios en las escrituras del WAL sobre `SimVfs`. `tests/process_kill.rs`: `kill -9` real de un escritor, 20 iteraciones por defecto (`KERNO_KILL_ITERS`) y 200 en la ejecución de salida de la Fase 1 ([informe F1](../testing/f1-report.md)).
- **K-E3-7** está en `kerno-units`, no en `kerno-storage`: las lecturas comprueban menos que `verify_units` (§2.3).

### 2.3 Fuera de alcance: se queda en la Etapa 9

- **IDs públicos** (ADR-0005, *Propuesto*, borrador) e I-27. Sin código en esta fase.
- **Cifrado y gestión de claves** (ADR-0006, *Pendiente*) e I-28. ADR-0006 prohíbe fijar supuestos de claves hasta aceptarlo.
- **Exportación e importación** (I-30). No hay código de exportación en `crates/`.
- **Migraciones con fixtures de versiones previas.** Dependen del estado del formato v1 (§2.4, punto 1).
- **Comparaciones con SQLite y umbrales frente a SQLite.** SQLite aparece solo en `spikes/storage/spike-bench/Cargo.toml`; ningún crate de `crates/` lo usa, y ningún E9A lo necesita. Los umbrales son del propietario (pendiente 3 del roadmap; ADR-0012).
- **Revisión de K-E3-7** («las lecturas comprueban menos que `verify_units`»). Las lecturas de unidades y relaciones están en `kerno-units/src/db.rs`, y la consulta asociativa de la Etapa 7 las usa: `assoc.rs:327` llama a `UnitSnapshot::reference_relations_of`. Revisarlas ahora mezclaría dos superficies en una sola revisión. **Recomendación:** hacerla en la Etapa 9, después de E7-06.
- **Backups y ventanas de expiración** (parte de ADR-0009). La decisión es del ADR; la implementación es de la Etapa 9.
- **Fuzzing con cobertura** (E9A-04): aprobado el 2026-11-11 (§7, punto 2); corre después de E9A-03.
- **Umbrales de latencia** en estrés o fuzzing. ADR-0012 sigue *Propuesto*; ningún E9A fija umbrales de latencia.
- **Escala `large` del benchmark** ([informe F1](../testing/f1-report.md) §5.1). Es una medición de la Fase 1, no una prueba de la Etapa 9.
- **Cambios de `Config::default`.** Ningún E9A cambia valores por defecto.

### 2.4 Discrepancias encontradas al leer (no corregidas aquí)

1. **Estado de ADR-0013 (resuelta el 2026-11-11, §7 punto 3).** La congelación se ejecutó el 2026-10-31 y ADR-0013 está aceptado desde esa fecha; las fuentes que decían lo contrario se corrigieron en la misma fecha. Texto original del punto: su propio texto dice «Aceptado (2026-10-31), condicionado a…». `docs/adr/README.md` dice «Propuesto (WP06; se acepta antes de congelar v1 en WP14)». El roadmap dice que la congelación del formato v1 está pendiente y que ADR-0013 «sigue Propuesto». [`crates/kerno-storage/DESIGN.md`](../../crates/kerno-storage/DESIGN.md) dice «congelado el 2026-10-31 (ADR-0013)» y que el bit `DRAFT` está retirado; [invariantes I-15](invariants.md) dice que, mientras la congelación esté pendiente, todo archivo lleva `DRAFT`. El propietario debe fijar un estado único antes de cualquier migración.
2. **Fase del fuzzing.** El roadmap lo pone en la Etapa 9. El [threat model §6.2](../security/threat-model.md) y el SAD (Fase 6) lo ponen en la Fase 6. Esta acta no decide la fase: trata el fuzzing con cobertura como puerta (E9A-04).
3. **Comentario de estrés.** El comentario de `group_commit.rs` dice «10^4 commits»; el código hace 4 × 1 500 y 4 × 1 000. Lo corrige E9A-05 si toca ese archivo; si no, un cambio de documentación aparte.

## 3. Paquetes de trabajo

Todos escriben solo en los archivos indicados. Ninguno cambia `src/` de `kerno-storage` ni `tests/common/` ni `tests/shared.rs`.

| WP | Contenido | Depende de | Escribe | Puede correr junto a |
|---|---|---|---|---|
| **E9A-00a** Criterios de ADR-0009 | Escribir en ADR-0009 los criterios de decisión del propio ADR (qué se elimina de inmediato, qué queda sujeto a compactación, qué plazo siguen los backups), con fecha, **antes** de medir nada | — | `docs/adr/0009-supresion-retencion-y-backups.md` (solo la sección de criterios) | todos |
| **E9A-01** Qué sobrevive a un borrado (solo medición) | Prueba de extremo a extremo: escribir valores con centinelas, borrar con `delete` y `delete_range`, y observar en cada etapa (antes de flush, tras `compact_range`, tras cierre limpio, tras reapertura) qué archivos del directorio siguen conteniendo el centinela. Sin política ni API nueva | E9A-00a | `crates/kerno-storage/tests/suppression_survival.rs` | E9A-02, E9A-03, E9A-05 |
| **E9A-02** I-29 en `kerno-storage` | Centinelas en claves y valores. Provocar cada variante de `Error`, `verify()` sobre daño, `inspect()` y `metrics()`. Comprobar que ningún `Display` ni `Debug` contiene un centinela ni su forma ASCII | — (ADR-0015 aceptado) | `crates/kerno-storage/tests/redaction.rs` | E9A-01, E9A-03, E9A-05 |
| **E9A-03** Propiedades de apertura | proptest sobre una base creada: muta un archivo (WAL, SST o MANIFEST) con bit flips, truncamientos y reescritura con CRC recalculado; abre en lectura-escritura y en solo lectura y llama a `verify()` | — | `crates/kerno-storage/tests/open_properties.rs` | E9A-01, E9A-02, E9A-05 |
| **E9A-04** Fuzzing con cobertura | Un objetivo por decodificador (14) y uno para la apertura, con cargo-fuzz en `crates/kerno-storage/fuzz/`, fuera del workspace, con corpus comprometido. No bloqueante en CI | E9A-03; dependencia y cadena de herramientas aprobadas el 2026-11-11 (§7, punto 2) | `crates/kerno-storage/fuzz/**` | — (corre después de E9A-03) |
| **E9A-05** Estrés del ciclo de vida | Ciclos de escrituras concurrentes (4 escritores), borrados y `delete_range`, `compact_range`, `sync()`, cierre limpio, reapertura y `verify()`. Comparación con el modelo en cada reapertura | — | `crates/kerno-storage/tests/lifecycle_soak.rs` | E9A-01, E9A-02, E9A-03 |
| **E9A-00b** Decisión sobre ADR-0009 (**hecho el 2026-11-12**, opción 3) | Elegir la opción 1, 2 o 3 y aceptar ADR-0009, o dejarlo pendiente y prohibir cualquier API de supresión | E9A-00a, E9A-01 | `docs/adr/0009-*.md`, `docs/adr/README.md` | — (decisión del propietario) |
| **E9A-06** Cierre (**hecho el 2026-11-12**, [cierre](etapa9-adelanto-cierre.md)) | Revisión adversarial de E9A-01…05. Promoción de I-29 para `kerno-storage` si E9A-02 pasa. Actualización de [invariantes](invariants.md) (I-27…I-30 siguen en «Fase 4»), [threat model](../security/threat-model.md) (T-06, T-08, T-09) y, si cambia lo que declara, [DESIGN.md](../../crates/kerno-storage/DESIGN.md) | E9A-00b, E9A-01…05 | `docs/architecture/etapa9-adelanto-cierre.md`, `invariants.md`, `threat-model.md` | — |

### 3.1 Qué corre a la vez

- **Ahora, junto con E7-02 a E7-06:** E9A-00a, E9A-01, E9A-02, E9A-03 y E9A-05. Cada uno crea un archivo nuevo en `crates/kerno-storage/tests/`. Ninguno modifica `tests/common/`, `tests/shared.rs` ni `src/`, así que no hay conflictos entre ellos.
- **Después de E9A-01 y de la decisión del propietario:** E9A-00b.
- **Después de E9A-03** (dependencia aprobada el 2026-11-11, §7, punto 2): E9A-04. No necesita a los demás.
- **Al final:** E9A-06, después de E9A-00b y de E9A-01 a E9A-05.
- **Fuera de todo paquete E9A:** `crates/kerno-units` y `crates/kerno-testkit`. Si un E9A necesita un ayudante de `kerno-testkit`, lo copia en su propio archivo; no lo añade a la crate compartida.
- **Hallazgo en `src/`:** si un E9A encuentra un fallo real en `kerno-storage/src`, lo registra como hallazgo y no lo corrige. La corrección es un paquete aparte, que se acepta por separado.

**Camino crítico para la Etapa 8:** E9A-00a → E9A-01 → E9A-00b, junto con E9A-02 → E9A-06.

### 3.2 Criterios de aceptación por paquete

Comunes a todos los paquetes con código: `cargo fmt --all --check`; `cargo clippy --workspace --all-targets --all-features -- -D warnings`; `cargo test -p kerno-storage --all-features --release` en verde; conteo de pruebas antes y después, con el archivo nuevo contado por el runner; cada prueba nueva vista fallar con una mutación aplicada a una **copia** del árbol, revertida y comprobada con `git diff`. Ninguna mutación queda en el árbol de trabajo. Clippy rechaza tipos de retorno complejos (`type_complexity`): usar un alias de tipo.

- **E9A-00a.** El texto de criterios está en ADR-0009 y su commit es anterior a cualquier ejecución de E9A-01. No contiene resultados.
- **E9A-01.** Antes de la primera ejecución, el archivo de pruebas lleva en su cabecera estas cuatro predicciones, que no se cambian después:
  - **P1.** Tras `delete` sin flush, el valor sigue en el WAL.
  - **P2.** Tras `FLUSH`, `compact_range` que cubre la clave y cierre limpio, el valor no aparece en ningún archivo vivo del directorio.
  - **P3.** La lápida conserva la clave en algún archivo vivo hasta que la compactación la descarta.
  - **P4.** Con `wal_preallocate = true`, un WAL volcado queda con ceros y el valor no aparece en él.

  Cada predicción pasa, o se registra como contradicha con la salida real. Ninguna se corrige para que pase. La prueba se etiqueta como **observación del comportamiento actual**, no como contrato, para que no fije una decisión de ADR-0009. Mutación de prueba: `compact_range` que no descarta valores cubiertos; P2 debe fallar.
- **E9A-02.** La prueba produce **las siete** variantes de `kerno_core::Error` (`Io`, `Corruption`, `Locked`, `InvalidArgument`, `Poisoned`, `Incompatible`, `ReadOnly`); si falta una, la prueba falla. No aparece ningún centinela, ni en bytes ni en su forma ASCII, en `Display` ni en `Debug`, en `Issue`, en `VerifyReport`, en `inspect()` ni en `metrics()`. Mutación de prueba: añadir la clave con `{:?}` al mensaje de `sst.rs:332`, con la tabla ausente, y la prueba debe fallar. Si E9A-02 pasa, I-29 queda promovida para `kerno-storage`, **pendiente de aprobación del propietario**.
- **E9A-03.** Cero pánicos. Cero aperturas que devuelvan datos distintos del modelo. Los únicos errores aceptados son `Corruption` e `Incompatible`. Mutación de prueba: quitar la comprobación de CRC de un bloque SST en la copia; la prueba debe fallar.
- **E9A-04.** Aprobado el 2026-11-11: un objetivo por decodificador más uno para la apertura, 30 minutos por objetivo en máquina local, cero fallos, cero entradas que tarden más de 1 s, corpus y entradas que fallan comprometidos en el repositorio.
- **E9A-05.** 200 ciclos por defecto (`KERNO_SOAK_CYCLES`), 4 escritores. Cero commits confirmados perdidos (la secuencia recuperada es al menos la última confirmada). `verify()` sin hallazgos en cada reapertura y contenido igual al modelo. Mutación de prueba: no escribir el checkpoint `CLEAN` en el cierre; la prueba debe fallar.
- **E9A-06.** Ningún hallazgo bloqueante abierto. El informe lleva los comandos y su salida real.

## 4. Criterios de aceptación de la etapa

| Criterio (roadmap, Etapa 9) | Cómo se cumple | Estado |
|---|---|---|
| Límites y garantías documentados | E9A-01 mide lo que sobrevive; E9A-06 lo escribe ([cierre](etapa9-adelanto-cierre.md)) | **hecho** (E9A-06) |
| Fallos críticos resueltos o marcados como bloqueantes | Cada hallazgo se registra en el [cierre](etapa9-adelanto-cierre.md) con su estado; ninguno bloqueante abierto | **hecho** (E9A-06) |
| I-29 verificada | E9A-02 (solo `kerno-storage`). Las observaciones de la Etapa 5 ya las cubre I-37 y ADR-0015 | **hecho para `kerno-storage`** (E9A-06, promovida por delegación del propietario el 2026-11-12, revocable). Las capas de logs siguen en Fase 4 |
| I-27, I-28, I-30 verificadas | Fuera de esta acta (§2.3) | pendiente, Etapa 9 |
| Fuzzing de entradas externas | E9A-03 en el árbol; E9A-04 con cobertura, aprobado el 2026-11-11. Umbral de 30 minutos por objetivo cumplido el 2026-11-12: 15 objetivos, cero fallos, ninguna entrada de más de 1 s ([resultados](../../crates/kerno-storage/fuzz/README.md)) | **hecho** (cierre en E9A-06) |
| Pruebas de estrés | E9A-05, además de las pruebas que ya existen (§2.2) | **hecho** (E9A-05; `tests/lifecycle_soak.rs` pasa en la suite de 2026-11-12, cierre en E9A-06) |
| ADR-0009 antes de la Etapa 8 | E9A-00b | **hecho el 2026-11-12**: ADR-0009 aceptado con la opción 3 |

## 5. Umbrales fijados antes de medir

| Paquete | Umbral |
|---|---|
| E9A-01 | Ninguno de rendimiento. Las predicciones P1 a P4 (§3.2) se fijan antes de ejecutar |
| E9A-02 | Siete de siete variantes de `Error` producidas; cero centinelas en cualquier texto |
| E9A-03 | 2 000 casos por tipo de archivo (WAL, SST, MANIFEST) y por modo (lectura-escritura, solo lectura); cero pánicos; cero datos distintos del modelo |
| E9A-04 | 30 minutos por objetivo; cero fallos; cero entradas de más de 1 s |
| E9A-05 | 200 ciclos; cero commits confirmados perdidos; cero hallazgos de `verify()` |

Estos umbrales los fija esta acta, por la delegación del 2026-11-11, y el propietario puede revocarlos. No se cambian después de medir. Un umbral que no se cumple se registra como incumplimiento; no se rebaja.

## 6. Invariantes

| ID | Qué hace el adelanto | Estado tras el paquete |
|---|---|---|
| I-29 | E9A-02 la verifica para `kerno-storage` | **Promovida para `kerno-storage` en E9A-06 (2026-11-12), aprobada por delegación del propietario, revocable**; sigue en Fase 4 para logs y auditoría |
| I-08 | E9A-03 la refuerza con entradas fabricadas al azar | Ya verificada; sin cambio de estado |
| I-02, I-09 | E9A-05 las refuerza con ciclos de cierre, reapertura y estrés | Ya verificadas; sin cambio de estado |
| I-27, I-28, I-30 | Fuera de esta acta | Pendientes (Fase 4) |

No existe invariante de supresión. Si ADR-0009 se acepta, la Etapa 8 propondrá la suya; esta acta no la inventa.

## 7. Decisiones que el propietario debe tomar

1. **Aceptar ADR-0009 (E9A-00b). Resuelto el 2026-11-12:** ADR-0009 se acepta con la opción 3, en los términos que recoge la [ADR](../adr/0009-supresion-retencion-y-backups.md). Hasta esa fecha no estaba incluida en la delegación del 2026-11-11, que cubría solo la pendiente 1, y sin decisión la Etapa 8 no entraba. Recomendación original: decidir después de E9A-01, con sus resultados delante, que es el orden seguido.
2. **Dependencia de fuzzing con cobertura (E9A-04): aprobada el 2026-11-11** en estos términos: cargo-fuzz con `libfuzzer-sys` y un toolchain nightly, solo en `crates/kerno-storage/fuzz`, fuera del workspace, no bloqueante en CI, y ejecutado después de E9A-03. Motivo de la pregunta original: cargo-fuzz exige `libfuzzer-sys` y un toolchain nightly, y eso es una dependencia nueva en el sentido de la escalera de `ponytail`. Recomendación original: decidirlo después de E9A-03, cuando se sepa qué cobertura falta (ya decidido).
3. **Estado único de ADR-0013** (§2.4, punto 1). **Resuelta el 2026-11-11:** ADR-0013 está **Aceptado** (2026-10-31) y el formato v1 está congelado desde ese día. Lo confirman el propio ADR-0013, `crates/kerno-storage/src/format/features.rs` y `header.rs` (rechazan el bit `DRAFT` como «pre-freeze draft format») y `crates/kerno-storage/DESIGN.md`. La fila de ADR-0013 en el índice de ADR, la línea del roadmap, I-15 y la cabecera de [`v1.md`](../file-format/v1.md) se corrigen en esa fecha.
4. **Cuándo revisar K-E3-7.** Recomendación: Etapa 9, después de E7-06 (§2.3).

## 8. Riesgos

| ID | Riesgo | Tratamiento |
|---|---|---|
| R-E9A-1 | ADR-0009 sin aceptar retrasa la Etapa 8 | **Resuelto el 2026-11-12** (E9A-00b). Ningún código de supresión en esta acta |
| R-E9A-2 | E9A-01 fija un comportamiento que ADR-0009 después cambia | Las pruebas se etiquetan como observación; las predicciones se registran antes de ejecutar |
| R-E9A-3 | I-29 se verifica sobre errores, verificación y métricas, pero `kerno-storage` no tiene capa de logs. Cuando la haya, la regla vuelve a aplicarse | E9A-02 se repite con cada capa nueva que emita texto |
| R-E9A-4 | Las pruebas aleatorias de semilla fija no crecen en cobertura | E9A-03 usa un caso por semilla propia y `PROPTEST_CASES`; E9A-04, aprobado el 2026-11-11 |
| R-E9A-5 | Estrés y fuzzing alargan la CI junto con la Etapa 7 | E9A-05 con 200 ciclos por defecto; no entra en la CI hasta medirlo |
| R-E9A-6 | T-09 sigue abierto aunque ADR-0009 se acepte: sin borrado físico garantizado, y el borrado criptográfico depende de ADR-0006 pendiente | Límite declarado en el threat model; no se promete borrado físico |
| R-E9A-7 | K-E3-7 queda sin revisar hasta la Etapa 9 | Aceptado por diseño el 2026-11-04 con recomendación de revisión en la Etapa 9 |

## 9. Lo que esta acta no decide

Las decisiones de §2.3 y §7 quedan en sus dueños. Esta acta no cambia ningún ADR, invariante, umbral de benchmark ni valor por defecto, y no toca código.

## 10. Aprobación

| Fecha | Decisión | Notas |
|---|---|---|
| 2026-11-11 | **Aprobada por delegación del propietario, revocable** | El propietario delegó la decisión pendiente 1 del roadmap y pidió el trabajo de endurecimiento en `kerno-storage` que no dependa de las Etapas 7 ni 8, en paralelo con la Etapa 7. Lo que se decide aquí: lo que afecta a datos derivados (ADR-0009 en su parte de criterios, I-29) se adelanta; el fuzzing, el estrés y las propiedades de apertura son los endurecimientos de `kerno-storage` que corren ahora. El resto queda en la Etapa 9. ADR-0009 no se acepta por esta delegación (§7, punto 1) |
| 2026-11-11 | **Decisiones del propietario:** fuzzing con cobertura aprobado en los términos de §7, punto 2 (E9A-04 después de E9A-03); estado de ADR-0013 resuelto (§7, punto 3) | ADR-0009 sigue sin decidir (§7, punto 1) |
| 2026-11-12 | **E9A-00b hecho: ADR-0009 aceptado con la opción 3** | Decisión 1 de §7 resuelta. Términos y relación con los criterios en [ADR-0009](../adr/0009-supresion-retencion-y-backups.md). T-09 sigue abierto ([threat model §6.5](../security/threat-model.md)) |
| 2026-11-12 | **E9A-06 hecho: cierre del adelanto** | Revisión adversarial de E9A-01 a E9A-05 en [cierre](etapa9-adelanto-cierre.md). Ningún hallazgo bloqueante abierto. I-29 promovida para `kerno-storage` por delegación del propietario, revocable. Actualizados invariantes, threat model (T-06, T-08, T-09), DESIGN.md (estado de ADR-0009) y roadmap |
