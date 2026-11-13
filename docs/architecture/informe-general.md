# Informe general de Kerno

| Campo | Valor |
|---|---|
| Fecha | 2026-11-13 |
| Estado del documento | Refleja el árbol después de los cuatro paquetes posteriores a la Etapa 9 (P1 a P4). Fecha 2026-11-13. **No se ejecutó ninguna prueba ni ninguna compilación para escribirlo**: las cifras de cada paquete son las de sus documentos, con su fecha |
| Árbol de referencia | commit `758b5d4`. Los cuatro paquetes están en la historia de este commit |
| Qué es | Una síntesis de lo que dicen los documentos del repositorio. No añade medidas ni pruebas nuevas. Donde el repositorio no tiene información, lo dice con esas palabras |
| Cómo se leyó | En esta actualización se leyeron completos los documentos de P1 a P4 ([post9-pendientes](post9-pendientes.md), [post9-mutaciones](post9-mutaciones.md), [requisitos-1.0](requisitos-1.0.md), [cla-propuesta](../legal/cla-propuesta.md), [units-v4](../file-format/units-v4.md), [ADR-0016](../adr/0016-adaptacion-e-invalidacion.md), [ADR-0005](../adr/0005-identificadores-publicos.md) y [ADR-0019](../adr/0019-licencia-y-contribuciones.md)). Además se leyeron [ADR-0020](../adr/0020-persistencia-de-vectores-como-derivados.md) en su criterio T5, la [acta de la Etapa 9](etapa9-acta.md) en §2.4 y §8.1, y [etapa9-fixtures](etapa9-fixtures.md) §3. Las afirmaciones sobre el código se comprobaron con búsquedas de texto y lectura de los archivos citados. Del resto de la versión anterior (cierres, actas, mediciones, roadmap, índice de ADR, CI) se conservan las citas, sin releerlas completas. No se leyeron `docs/research/` ni `experiments/`, que quedan fuera de esta actualización |

> **Estado de los paquetes posteriores a la Etapa 9.** Este informe refleja el árbol después de los cuatro paquetes: P1 (K1 y formato de unidades v4), P2 (pruebas que faltaban, un mensaje de error y una prueba intermitente), P3 (las tres carencias de ADR-0016) y P4 (auditoría de requisitos de 1.0, borrador de CLA sin vigor y registro de la aleatoriedad de los identificadores). Está escrito a 2026-11-13. Las cifras de cada paquete son las que da su documento, con su fecha. **Ninguna prueba se ejecutó para escribir este informe**, porque la tarea prohibía compilar. Las dos inconsistencias documentales que P4 encontró y que nadie corrigió están en la sección 5.9.

## Convenciones de este documento

**Etapas y fases.** La especificación numera el trabajo en etapas 0 a 9. El SAD numera las fases 0 a 6. La tabla de correspondencia está en [`docs/spec/README.md`](../spec/README.md). Aquí se usa la numeración de etapas. «Etapa 0» es la fase M0 del SAD. «Etapa 2» es la Fase 1 del SAD, el núcleo persistente.

**Etiquetas que aparecen en los documentos.** Cada documento usa sus propias etiquetas. Estas son las que aparecen aquí:

| Etiqueta | Qué es | Dónde se define |
|---|---|---|
| ADR-00NN | Registro de decisión de arquitectura: una decisión duradera, con contexto, opciones y evidencia | [índice de ADR](../adr/README.md) |
| I-NN (I-01 a I-45) | Invariante: algo que ningún cambio puede romper, y la prueba que lo verifica | [`invariants.md`](invariants.md) |
| UNIT-NN | Regla de verificación de la capa de unidades, que aplica `verify_units` | [`units-v1.md`](../file-format/units-v1.md) y siguientes |
| K1 a K5 | Hallazgos de la revisión de las lecturas de `kerno-units` frente a `verify_units` (K-E3-7). K1 es que una relación con una sola entrada de adyacencia es invisible desde el extremo que la pierde | [revisión K-E3-7](etapa9-k-e3-7.md) |
| K-E3-1 a K-E3-9 | Riesgos abiertos de la Etapa 3, heredados por las etapas siguientes | [cierre de la Etapa 3](etapa3-cierre.md) |
| H-1 a H-14, F-01 a F-15, R-1 a R-12 | Hallazgos de revisión de cada etapa. Cada cierre usa su letra | Cada cierre de etapa, en su sección de revisión |
| D1 a D23 | Decisiones del acta de la Etapa 9. Su tabla está en [acta §8.1](etapa9-acta.md) | [acta de la Etapa 9](etapa9-acta.md) |
| E9-02, E8-10, E7-05… | Paquetes de trabajo de cada etapa | Cada acta |
| T1 a T10 | Objetivos de rendimiento de la Fase 1 | [`f1-objetivos-rendimiento.md`](../testing/f1-objetivos-rendimiento.md) |
| U1 a U6 | Objetivos de rendimiento de la Etapa 3 | [`etapa3-report.md`](../testing/etapa3-report.md) |
| Q1 a Q6, P1 a P4 de la Etapa 8 | Criterios de calidad (Q) y de tiempo (P) de la Etapa 8. **P1 y P2 aquí son cocientes de tiempo de la Etapa 8, no los paquetes posteriores** | [acta de la Etapa 8](etapa8-acta.md) |
| P1 a P4 posteriores | Paquetes posteriores a la Etapa 9, ya en el árbol. Ver la sección 2 | Este documento |

**Términos del motor.** El motor de almacenamiento es un árbol de fusión por niveles, llamado LSM. Su registro de escritura anticipada es el WAL. Su registro de archivos vivos es el MANIFEST. Sus tablas ordenadas en disco son las SST. Un commit tiene dos modos de durabilidad: `Sync` (sincroniza en cada commit) y `Buffered` (no sincroniza en cada commit). Las definiciones están en [ADR-0002](../adr/0002-transacciones-y-durabilidad.md) y en [v1.md](../file-format/v1.md).

**Términos de la capa de unidades.** Una unidad (`Unit`) tiene identidad, tipo y revisiones. Un `ChangeSet` agrupa las escrituras de un commit. Una relación declarada une dos unidades con un tipo. Un índice de adyacencia permite recorrer las relaciones de una unidad. Un derivado es un dato calculado a partir de unidades, con sus fuentes registradas. Un vector es un derivado con un número fijo de componentes. Un resumen es un derivado de texto con fuentes obligatorias.

**Dos motores en las pruebas.** Las pruebas de `kerno-units` se ejecutan dos veces: sobre `emulated_log` (el `LogEngine` de `kerno-testkit`, [`reference.rs`](../../crates/kerno-testkit/src/reference.rs)), y sobre `kerno` (`KernoDb`, el motor real). Su documentación dice que `LogEngine` guarda todo el estado en memoria y un registro de lotes con sumas de control en disco, y que existe para validar el arnés de pruebas: si él falla una prueba compartida, la prueba está mal. Las pruebas que citan «en los dos motores» hacen eso.

---

## 1. Resumen en una página

**Qué es Kerno.** Kerno es un motor de datos embebido y local-first. Combina persistencia transaccional, evolución versionada de los datos e integridad verificable. Tiene una ruta de extensión hacia la recuperación híbrida para aplicaciones de IA. Está escrito en Rust. Su código se reparte en cuatro crates: `kerno-core` (el contrato compartido), `kerno-storage` (el motor), `kerno-testkit` (la infraestructura de pruebas y el motor de referencia) y `kerno-units` (la capa de unidades, revisiones, relaciones, índices, planificador y derivados). Fuente: [README](../../README.md) y [SAD §14](SAD.md#14-organización-propuesta-del-repositorio).

**En qué estado está hoy.**

- **No hay versión 1.0.** El README lo dice en su primera sección de estado. No hay promesa de estabilidad del formato ni de la API. Fuente: [README](../../README.md).
- **El formato en disco v1 está congelado** desde el 2026-10-31, con [ADR-0013](../adr/0013-formato-v1-y-protocolo-del-motor.md). Eso no es una promesa de compatibilidad para usuarios.
- **Las etapas 3 a 9 están cerradas**, con matices. La Etapa 8 está cerrada «como trabajo de ingeniería». Su criterio Q4, calidad de recuperación con y sin señal vectorial, **no está demostrado**. La Etapa 9 tiene su cierre aprobado por el propietario el 2026-11-13. Fuentes: [roadmap](../roadmap/README.md), [cierre de la Etapa 8](etapa8-cierre.md), [cierre de la Etapa 9](etapa9-cierre.md).
- **Los paquetes posteriores a la Etapa 9 están en el árbol.** P1 cierra K1 para las relaciones con registro de conteo. P2 añade pruebas y corrige un mensaje y una prueba intermitente. P3 cierra las tres carencias de ADR-0016. P4 escribe la auditoría de requisitos de 1.0, un borrador de CLA sin vigor y el registro de la aleatoriedad de los identificadores. Detalle en la sección 2. Fuentes: [post9-pendientes](post9-pendientes.md), [post9-mutaciones](post9-mutaciones.md), [requisitos-1.0](requisitos-1.0.md).
- **No se ha declarado 1.0.** Eso depende de D13 y de un uso real fuera del equipo (D12). Fuente: [acta de la Etapa 9 §8.1](etapa9-acta.md).

**Lo que más importa, en cuatro puntos.**

1. **Lo que se ha probado es mucho, pero se ha probado en un solo tipo de máquina.** Hay pruebas de caídas en cada punto de E/S, de corrupción, de `kill -9` real, un barrido de 1 000 semillas, quince objetivos de fuzzing y 856 pruebas en `kerno-units` y `kerno-testkit` (cifra del 2026-11-12). Tras P3, la lista de pruebas de `kerno-units` tiene 870 (2026-11-13; es un conteo de la lista, no una ejecución completa). Todo sale de una VM de cuatro núcleos con Linux y ext4. Ninguna medición se hizo en la máquina objetivo, que el repositorio no nombra (C-09). Fuentes: [informe de la Fase 1 §4](../testing/f1-report.md), [cierre de la Etapa 9 §5](etapa9-cierre.md).
2. **Varios objetivos de rendimiento no se cumplen, y eso está aceptado y escrito.** En la Fase 1, T1 (escritura de `agent_memory`, 2,78 veces la referencia LSM frente a un máximo de 2,0) y T7 (amplificación de escritura, 4,29 frente a 1,20) no se cumplen. En la Etapa 3, U2 y U6 no se cumplen; el propietario lo aceptó como riesgo abierto, sin rebajar umbrales. Fuentes: [informe de la Fase 1 §5.3](../testing/f1-report.md), [cierre de la Etapa 3 §4](etapa3-cierre.md).
3. **Hay límites de integridad y de seguridad que no están construidos.** No hay cifrado en reposo (D3) ni integridad autenticada (D4). Un filtro de una SST reescrito con un CRC válido puede pasar la apertura y responder «no existe» sin error para claves que sí están. La apertura rechaza el caso grueso, pero solo `verify()` con `deep` informa el caso de claves interiores. No hay borrado físico garantizado: el borrado es lógico y la reclamación pública es un mecanismo, no un borrado seguro. Fuentes: [threat model §3 y §6](../security/threat-model.md), [ADR-0009](../adr/0009-supresion-retencion-y-backups.md).
4. **Algunas capacidades quedan apagadas o sin demostrar a propósito.** El ranking de resultados asociativos está desactivado por defecto y es experimental. La caché de resultados está desactivada por defecto, y su mejora neta no se afirma. Con la caché encendida, no guarda nada mientras el observador esté por debajo de `Fingerprint` (P3, sección 4.9). La calidad de recuperación con vectores no está demostrada, y los vectores de las pruebas son deterministas con semilla. Fuentes: [cierre de la Etapa 7 §8](etapa7-cierre.md), [cierre de la Etapa 6 §0](etapa6-cierre.md), [cierre de la Etapa 8 §0.3](etapa8-cierre.md).

**Lo que conviene hacer primero, en este orden.**

1. **Revisar P1 antes de darlo por cerrado.** El defecto que halló la revisión independiente (contadores huérfanos en una creación de relación rechazada) está corregido y cubierto por la mutación M20 (sección 4.7). Pero [post9-pendientes](post9-pendientes.md) sigue dando la revisión independiente de P1 como pendiente, y P1 toca el formato de disco.
2. **Clonar el repositorio en una máquina propia y ejecutar la batería de la sección 9** (formato, clippy, pruebas de los dos crates, guardas de dependencias). Los documentos no dan duraciones completas. La sección 9 dice qué se sabe.
3. **Repetir en reposo las mediciones de tiempo de la Etapa 8 (P1 y P2, cocientes)**, y la corrida de `wal`, `manifest` y `open` del fuzzing con más tiempo. Ambas cosas se dejaron a propósito para la máquina del propietario, porque el disco de la máquina de compilación estaba casi lleno y es compartido. La sección 9 da los comandos.
4. **Decidir el texto del contrato de contribución (CLA) antes de aceptar cualquier contribución externa.** Hay un borrador sin vigor ([cla-propuesta](../legal/cla-propuesta.md)), que necesita la revisión de un abogado y la aprobación del propietario (D15). Fuente: [ADR-0019](../adr/0019-licencia-y-contribuciones.md).
5. **No declarar 1.0** hasta cumplir D13 y tener un uso real fuera del equipo (D12). La lista de requisitos y su estado están en la sección 8.
6. **Corregir o decidir las dos inconsistencias de la sección 5.9.** Una está en el acta de la Etapa 9, cerrada, y la otra en ADR-0020, aceptado.

---

## 2. Qué se ha logrado

La tabla resume cada etapa. Las fechas son las de cada cierre. Cuando un cierre fue aprobado por delegación del propietario, se dice.

| Etapa (SAD) | Fecha de cierre | Qué construyó | Dónde está documentado |
|---|---|---|---|
| **0 / M0** (Fase 0) | 2026-10-28 | Dos prototipos de almacenamiento, un árbol B+ copy-on-write y un LSM nivelado, con las mismas pruebas y los mismos benchmarks. Se eligió el LSM (ADR-0001). ADR-0002 (transacciones y durabilidad) y ADR-0003 (formato y versionado) aceptados el mismo día. Matriz de fallos. Benchmarks de 410 ejecuciones | [m0-report](../testing/m0-report.md), [ADR-0001](../adr/0001-motor-de-almacenamiento.md), [ADR-0002](../adr/0002-transacciones-y-durabilidad.md), [ADR-0003](../adr/0003-formato-fisico-y-versionado.md), [matriz de fallos](failure-matrix.md), [spikes/storage](../../spikes/storage/README.md) |
| **1** (contratos mínimos) | Parcial | Contratos de almacenamiento y de transacción definidos. Unidad, revisión y relación pasaron a la Etapa 3. La propia tabla de la especificación lo marca como «Parcial» | [docs/spec/README.md](../spec/README.md) |
| **2 / Fase 1** (núcleo persistente) | Cerrada según el roadmap, con fecha 2026-11-11 | `kerno-core` y `kerno-storage`: formato v1 (congelado con ADR-0013, aceptado el 2026-10-31), motor LSM, WAL, MANIFEST como log de ediciones, borrados de rango, lectura de solo lectura, group commit, transacciones read-modify-write. Revisión adversarial con defectos corregidos (sección 4) | [roadmap](../roadmap/README.md), [informe de la Fase 1](../testing/f1-report.md), [revisión adversarial](../testing/f1-adversarial-review.md), [v1.md](../file-format/v1.md), [ADR-0013](../adr/0013-formato-v1-y-protocolo-del-motor.md) |
| **3** Unidades, revisiones y procedencia | 2026-11-04 (aprobada por el propietario) | `kerno-units`: unidades con ciclo de vida, `ChangeSet`, revisiones, historial, `restore`, políticas `none`, `audit` y `versioned`, retención por lotes, `verify_units`. Dos objetivos de rendimiento (U2 y U6) no se cumplen y están aceptados como riesgo | [cierre](etapa3-cierre.md), [etapa3-report](../testing/etapa3-report.md), [units-v1](../file-format/units-v1.md), [diseño](etapa3-design.md), [ADR-0004](../adr/0004-historial-de-revisiones.md), [ADR-0010](../adr/0010-api-publica-y-crates.md) |
| **4** Relaciones e índices | 2026-11-05 (aprobada por el propietario) | Relaciones declaradas (`REL` y dos índices de adyacencia, ADR-0014), índice primario con estado y marca de agua, índice por tipo `IDX_TYPE` justificado por la carga W1, ruta de referencia sin índices para todas las consultas | [cierre](etapa4-cierre.md), [acta](etapa4-acta.md), [cargas](etapa4-workloads.md), [ADR-0007](../adr/0007-consistencia-de-indices.md), [ADR-0014](../adr/0014-representacion-fisica-de-relaciones.md) |
| **5** Planificador y observabilidad | 2026-11-06; aprobada por delegación el 2026-11-09 | Planificador determinista de siete pasos, con explicación del plan. `ExecutionObservation` con cuatro niveles y retención acotada. Centinelas que prueban que no hay literales fuera del nivel `diagnostic` | [cierre](etapa5-cierre.md), [planificador](etapa5-planificador.md), [observación](etapa5-observacion.md), [centinelas](etapa5-centinelas.md), [ADR-0015](../adr/0015-privacidad-de-la-observabilidad.md), [ejecución adaptativa](adaptive-execution.md) |
| **6** Primera plasticidad operacional | 2026-11-11; aprobada por delegación | Una sola optimización: una caché de resultados ligada a `commit_seq`, usada por el planificador con un fallback registrado. **Desactivada por defecto.** Mejora neta no afirmada | [cierre](etapa6-cierre.md), [medición](etapa6-medicion.md), [reversibilidad](etapa6-reversibilidad.md), [ADR-0016](../adr/0016-adaptacion-e-invalidacion.md) |
| **7** Memoria asociativa | 2026-11-12; aprobada por el propietario | Consulta asociativa con cotas duras (profundidad 4, 1 000 resultados, 100 000 visitas), ejecución por índice con comprobación de cada respuesta, rastro verificable de cada resultado (I-40), ranking con modo diagnóstico (I-41). **El ranking está desactivado por defecto y es experimental.** El conjunto de evaluación cumple T1 a T3 y T5, pero no T4 | [cierre](etapa7-cierre.md), [consulta](etapa7-consulta-asociativa.md), [ejecución indexada](etapa7-ejecucion-indexada.md), [contexto y procedencia](etapa7-contexto-y-procedencia.md), [ranking](etapa7-ranking.md), [medición](etapa7-medicion.md), [conjunto](etapa7-conjunto-evaluacion.md), [ADR-0011](../adr/0011-vectores-y-recuperacion-hibrida.md) |
| **8** Consolidación y embeddings | 2026-11-12, cerrada como trabajo de ingeniería | Registro común de derivados con estados `ready`, `building`, `stale`, `failed` y `superseded` (ADR-0018). Obsolescencia y reconstrucción en el mismo commit que el cambio de fuente. Vectores persistidos como derivados (ADR-0020). Supresión de fuentes y derivados (ADR-0009). Consolidación con resúmenes de procedencia obligatoria y un resumidor de prueba determinista. **Q4 no demostrada.** Un paquete de seguimiento (E8-10) es necesario antes de 1.0 | [cierre](etapa8-cierre.md), [acta](etapa8-acta.md), [conjunto](etapa8-conjunto-evaluacion.md), [medición](etapa8-medicion.md), [ADR-0018](../adr/0018-versionado-y-reconstruccion-de-derivados.md), [ADR-0020](../adr/0020-persistencia-de-vectores-como-derivados.md), [ADR-0009](../adr/0009-supresion-retencion-y-backups.md) |
| **9** Endurecimiento | 2026-11-13; cierre aprobado por el propietario en persona | Identificadores públicos con token opaco (E9-02, ADR-0005, fusionado el 2026-11-13). Auditoría minimizada (E9-03). Exportación e importación con formato borrador sin congelar (E9-04 a E9-06). Reclamación pública `Reclaim` (E9-07). Retirada de `UnitDb::engine()` (E9-08). Lectura de derivados, estrés, decodificadores robustos, fuzzing de quince objetivos, mutaciones de la Etapa 8, revisión de K-E3-7, informe SQLite nuevo, conjunto y medición de E8-10, ejemplo de uso real `agent-memory` con quince puntos de fricción | [cierre](etapa9-cierre.md), [acta](etapa9-acta.md), [adelanto](etapa9-adelanto-cierre.md), [auditoría](etapa9-auditoria.md), [fixtures](etapa9-fixtures.md), [revisión K-E3-7](etapa9-k-e3-7.md), [usos de engine()](etapa9-usos-de-engine.md), [informe SQLite E9](../testing/sqlite-report-e9.md), [medición E9](etapa9-medicion.md), [FRICTION](../../examples/agent-memory/FRICTION.md), [ADR-0005](../adr/0005-identificadores-publicos.md), [ADR-0006](../adr/0006-cifrado-y-gestion-de-claves.md), [ADR-0012](../adr/0012-metodologia-de-benchmarks.md) |

**Paquetes posteriores a la Etapa 9** (estado según el árbol y los documentos de cada paquete; los commits están en la historia de `758b5d4`):

| Paquete | Estado | Qué es | Dónde está documentado |
|---|---|---|---|
| **P1** | **Entregado** (`0c2c3d5`, con la corrección `4b65c51` y `0310bb3`) | Cierra K1 para las relaciones con registro de conteo: formato de unidades v4, con un registro por `(unidad, dirección, tipo)` que el lector compara con las entradas de adyacencia y que `verify_units` comprueba (UNIT-45 y UNIT-46). Una revisión independiente halló un defecto de la creación rechazada, corregido y cubierto por la mutación M20 (sección 4.7). 20 mutaciones (M1 a M20), todas en rojo | [units-v4](../file-format/units-v4.md), [post9-mutaciones](post9-mutaciones.md), [post9-pendientes](post9-pendientes.md) |
| **P2** | **Entregado** (`2356ff3`) | Pruebas que faltaban (cierre de la Etapa 9, puntos 4, 5, 6 y 10), el mensaje de `NoValidSources`, y la prueba intermitente `space_is_reclaimed_after_range_deletes`, cuya causa fue una suposición de temporización de la propia prueba (sección 4.8). 18 registros de mutación: Q1 a Q16, más Q13b (candidato descartado, en verde) y Q13c | [post9-mutaciones](post9-mutaciones.md) (registro P2), [post9-pendientes](post9-pendientes.md) (puntos 3 a 6, 10 y 11) |
| **P3** | **Entregado** (`fc7e233`) | Las tres carencias de ADR-0016 («trabajo abierto» 1 a 3): la explicación del plan dice si se consultó la caché, la evidencia de creación es la del observador y la entrada lleva un coste medido, y `set_key` deja un registro. Cambio de comportamiento: con el observador por debajo de `Fingerprint`, la caché no guarda nada (sección 4.9). 17 mutaciones (M1 a M17), todas en rojo | [ADR-0016, «Revisión de 2026-11-13»](../adr/0016-adaptacion-e-invalidacion.md), [post9-mutaciones](post9-mutaciones.md) (registro P3) |
| **P4** | **Entregado** (`f2aa157`, solo documentos) | Auditoría de los siete requisitos de D13: seis con límites declarados y uno cumplido; 1.0 no se declara. Borrador de CLA en español e inglés, **sin vigor**. Registro de la aleatoriedad de los identificadores públicos (`/dev/urandom`, sin dependencia nueva). Dos inconsistencias documentales que no se corrigieron (sección 5.9) | [requisitos-1.0](requisitos-1.0.md), [cla-propuesta](../legal/cla-propuesta.md), [ADR-0005, «Notas»](../adr/0005-identificadores-publicos.md), [post9-pendientes](post9-pendientes.md) |

---

## 3. Qué se ha probado y cómo

Esta sección separa cada tipo de evidencia. Para cada uno dice qué cubre y qué no. Las cifras van con la fecha del documento que las da. **El repositorio no tiene un conteo total de pruebas después de la Etapa 9.** El cierre de la Etapa 9 dice que no reejecutó el conteo (T-CNT).

### 3.1 Conteos de pruebas que dan los documentos

| Ámbito | Cifra | Fecha | Fuente |
|---|---|---|---|
| `kerno-units` y `kerno-testkit`, release, todas las funciones | 856 aprobadas, 0 fallidas, 8 ignoradas | 2026-11-12 | [cierre E8 §9.1](etapa8-cierre.md) |
| `kerno-units` y `kerno-testkit`, con el cierre de la Etapa 7 | 645 aprobadas, 0 fallidas, 8 ignoradas | 2026-11-12 | [cierre E7 §5](etapa7-cierre.md) |
| `kerno-units` solo, release | 320 aprobadas, 1 ignorada | 2026-11-11 | [cierre E6 §5](etapa6-cierre.md) |
| `kerno-storage`, release | 649 aprobadas, 4 ignoradas | 2026-11-06 | [cierre E5 §4](etapa5-cierre.md) |
| Workspace sin `kerno-storage`, release | 615 aprobadas, 8 ignoradas | 2026-11-06 | [cierre E5 §4](etapa5-cierre.md) |
| Workspace completo | 1 212 antes de E4-05 y 1 214 después | 2026-11-05 | [cierre E4 §2](etapa4-cierre.md) |
| `kerno-units`, lista de pruebas (`--list`) | 822 antes de P1 y 853 después (+31 en `rel_counts.rs`) | 2026-11-13 | [post9-mutaciones](post9-mutaciones.md) |
| `kerno-core`, `kerno-storage`, `kerno-testkit` y `kerno-units`, lista de pruebas | 1 798 en el árbol base de P2 y 1 804 con P2 (+6: `reclaim.rs`, `db.rs` y `derived_lock_order.rs`) | 2026-11-13 | [post9-mutaciones](post9-mutaciones.md), registro P2 |
| `kerno-units`, lista de pruebas (`--list`) | 859 antes de P3 y 870 después (+3 en `cache.rs`, +8 en `cache_planner.rs`) | 2026-11-13 | [post9-mutaciones](post9-mutaciones.md), registro P3 |
| `kerno-units`, E9-02 | 49 pruebas sin `test-support` y 79 con ella, en los dos motores | 2026-11-13 | [cierre E9 §1](etapa9-cierre.md) |
| Storage de M0, prototipo B+ tree | 32 pruebas unitarias propias | 2026-10-27 | [m0-report §2.1](../testing/m0-report.md) |
| Storage de M0, prototipo LSM | 24 unitarias y 7 de integración | 2026-10-27 | [m0-report §2.1](../testing/m0-report.md) |

Las cifras no son comparables entre sí. Cada una cuenta un ámbito distinto. Una prueba que ya no existe deja de contar. Por ejemplo, `completeness_is_not_a_query_time_check` pasó a llamarse `a_relation_lost_with_both_entries_is_caught_by_its_counts` en P1, y el cierre de la Etapa 7 todavía cita el nombre antiguo ([post9-pendientes](post9-pendientes.md)).

### 3.2 Pruebas unitarias e integración de los dos motores

- **Qué son.** Pruebas en `crates/kerno-storage/tests/` (42 entradas, con subcarpetas), `crates/kerno-units/tests/` (54 entradas, con subcarpetas) y `crates/kerno-testkit/tests/` (5 ficheros). Cada prueba de `kerno-units` que habla de datos corre en `emulated_log` y en `kerno`.
- **Qué cubren.** Las reglas `UNIT-n`, los códigos de cada registro, la lectura y la escritura de unidades, revisiones, relaciones y sus registros de conteo (P1, `rel_counts.rs`), índices, la caché, el planificador, los derivados, la exportación y la importación, la reclamación pública y los decodificadores. Los vectores de bytes (`tests/vectors/`) fijan la codificación de cada versión del formato.
- **Qué no cubren.** Bases sin registro de conteo (de antes de la capa 4) con más de 64 relaciones y una entrada ausente en ambos extremos (H-1 de la Etapa 7, sección 5). Para las relaciones con registro de conteo, P1 añade pruebas en los dos motores (`rel_counts.rs`). Ninguna prueba de concurrencia entre lectores de la caché (H-1 de la Etapa 6, sección 3.4 de su cierre). Ninguna prueba en plataformas distintas de Linux con ext4 (ver sección 6).

### 3.3 Prueba diferencial contra el motor de referencia

- **Qué es.** Se ejecuta la misma historia de operaciones sobre el motor real y sobre el motor de referencia en memoria (`emulated_log`), o sobre la ruta de referencia sin índices, y se comparan las respuestas. Ejemplos: `differential_seeded` en `tests/index.rs` (seis semillas, dos motores, cuarenta pasos; desde E4-05 incluye `restore` y `run_retention`, [cierre E4 §2](etapa4-cierre.md)), `differential_across_scan_pages`, y las pruebas `differential_*` de `tests/relations.rs`.
- **Qué prueba la de planificador.** `cache_on_off_and_invalidated_give_identical_results` compara la respuesta con la caché encendida, apagada e invalidada, con escrituras entre rondas ([cierre E6 §2](etapa6-cierre.md)).
- **Corpus congelado.** Las 64 consultas del conjunto de la Etapa 7 dan la misma respuesta por la referencia, por el índice y por el planificador: 64 de 64, sin caídas ([cierre E7 §4.1](etapa7-cierre.md)). El conjunto tiene un `MANIFEST` con SHA-256 que se comprueba antes de medir.
- **Qué no cubre.** Que la ruta de índice sea completa en tiendas de 64 relaciones o más con una entrada ausente en ambos extremos, en bases sin registro de conteo. Para las relaciones con registro de conteo, P1 hace que sí se detecte en consulta (`Fallback` `Corrupt`, [units-v4 §5](../file-format/units-v4.md)). Sin conteo, solo lo ve `verify_units` (cierre E7, R-2). Tampoco cubre las tiendas dañadas, salvo en las pruebas que las dañan a propósito.

### 3.4 Caídas e inyección de fallos

- **Simulador.** `SimVfs` en [`crates/kerno-testkit/src/vfs/sim.rs`](../../crates/kerno-testkit/src/vfs/sim.rs) simula cortes de energía en cada punto de E/S, muertes de proceso, desgarros de escritura, truncamientos y errores de `fsync`. La matriz de fallos ([failure-matrix](failure-matrix.md)) dice qué debe quedar tras reabrir en cada punto.
- **Barridos.** Caída antes de cada operación de E/S, un EIO aislado en cada operación, y el modo de `fsync` fallido con semántica de Linux, en `Sync` y en `Buffered` (M0 §2.1, [informe de la Fase 1 §2](../testing/f1-report.md)).
- **Mil semillas (2026-10-29).** En la variante por defecto del motor de producto: 1 000 truncamientos detectados al abrir, 0 inocuos, 0 datos erróneos. Bit flips: 256 detectados al abrir, 744 al leer, 0 inocuos, 0 datos erróneos ([informe de la Fase 1 §2](../testing/f1-report.md)).
- **`kill -9` real (2026-10-29).** 200 muertes de un escritor en `Sync` y 200 en `Buffered`, las dos pruebas en verde. Dos pruebas más con cuatro escritores concurrentes, añadidas después de ese informe, con su salida en [`process-kill-200.log`](../../spikes/storage/results/2026-10-29-f1/process-kill-200.log).
- **Qué no cubre.** Un dispositivo real que pierda páginas. El registro de LazyFS está **pendiente**: el informe de la Fase 1 dice que no hay ninguna ejecución registrada ([§4](../testing/f1-report.md)). El simulador modelaba mal el fallo de `wait_writeback` (G1), y eso se corrigió (sección 4). El modelo de `kill -9` no pierde la caché del sistema operativo, y el propio documento lo dice ([failure-matrix §1](failure-matrix.md)). Los modelos de `loom` solo corren en la CI semanal, y los documentos no dan un registro de su resultado.

### 3.5 Fuzzing

Quince objetivos sobre los decodificadores de `kerno-storage` y la apertura de una base. Están en [`crates/kerno-storage/fuzz/`](../../crates/kerno-storage/fuzz/README.md), con su propio espacio de trabajo. No corren en la CI.

Hay tres tandas documentadas:

| Tanda | Fecha | Duración por objetivo | Resultado |
|---|---|---|---|
| Corta, con el corpus comprometido | 2026-11-10 | 60 s | Sin fallos, sin entradas de más de 1 s |
| E9A-04, primera tanda de treinta minutos | 2026-11-12 | 1 801 s | Sin fallos en los quince |
| E9-14, sobre el código final de la Etapa 9 | 2026-11-12 | 1 801 s, cuatro objetivos a la vez | Sin fallos en los quince, sin entradas nuevas en el corpus |

Resultados de la tanda E9-14, con la carga de la máquina alrededor de 4 en 4 CPU ([cierre E9 §5, punto 7](etapa9-cierre.md)):

| Objetivo | Ejecuciones en 30 min | Fallos |
|---|---:|---:|
| varint | 427 959 903 | 0 |
| header | 296 364 723 | 0 |
| commit | 207 625 874 | 0 |
| frame | 313 543 961 | 0 |
| data_block | 198 919 045 | 0 |
| filter_block | 262 139 947 | 0 |
| rangedel_block | 175 649 868 | 0 |
| props_block | 242 636 008 | 0 |
| index_block | 183 972 923 | 0 |
| footer | 499 398 250 | 0 |
| record | 310 873 073 | 0 |
| **wal** | **4 877 579** | 0 |
| sst | 198 623 700 | 0 |
| **manifest** | **6 676 172** | 0 |
| **open** | **4 546 088** | 0 |

Fuente: [`fuzz/README.md`](../../crates/kerno-storage/fuzz/README.md), sección «Stage 9 run on the final code». La primera tanda de treinta minutos (E9A-04) dio cifras del mismo orden para `wal` (5 698 047), `manifest` (7 420 267) y `open` (4 959 653). Los objetivos `wal`, `manifest` y `open` hacen trabajo de archivos por entrada. Su número de ejecuciones es de dos órdenes de magnitud menos que el de los decodificadores puros. El cierre dice que treinta minutos es poco para ellos. **El fuzzing de `kerno-units` no existe**: D21 decidió usar solo pruebas de propiedades con la biblioteca que ya es dependencia.

Lo que no cubre: corridas largas de `wal`, `manifest` y `open` en máquina libre (pendiente, dejada para la máquina del propietario; sección 9.3), y cualquier entrada que no sea un archivo de base. El fuzzing de la CI no existe: el threat model dice que T-10 sigue abierta porque la CI no ejecuta fuzzing ni auditoría de dependencias ([threat-model §3](../security/threat-model.md)).

### 3.6 Mutaciones

Una mutación cambia una línea de producción o de prueba, y se comprueba que alguna prueba se pone en rojo. Cada paquete tiene su registro.

| Paquete | Mutaciones | Resultado que dice el documento | Fuente |
|---|---|---|---|
| Harness de M0 (validación) | 8 mutaciones a mano | Cada una detectada por una prueba. Dos eran errores reales de un motor | [test-plan §2.1](../testing/test-plan.md) |
| E3-10 (revisión de la Etapa 3) | Una por invariante IU-1 a IU-10 | Cada invariante con su prueba que falla al romperla; hallazgos H-1 a H-3 | [cierre E3 §2](etapa3-cierre.md) |
| E4-05 | Una por invariante de la etapa | Huecos H-1 a H-5 corregidos | [cierre E4 §3](etapa4-cierre.md) |
| E5-05 | 3 mutaciones de producción | Las tres detectadas. Una de ellas no la ve la búsqueda de sentinelas: la detecta la prueba estructural `levels_below_diagnostic_carry_no_shape_text` | [cierre E5 §3.1](etapa5-cierre.md) |
| E6-05 | 3 mutaciones de producción | Dos detectadas; **una sobrevive** (H-7: con la caché apagada, el código sigue tomando su camino, y ninguna prueba lo ve) | [cierre E6 §3.3](etapa6-cierre.md) |
| E7-02 | 29 mutaciones | Todas detectadas | [cierre E7 §0](etapa7-cierre.md) |
| E7-03 | 16 mutaciones | Todas detectadas | [cierre E7 §0](etapa7-cierre.md) |
| E7-04 | 42 mutaciones (M01 a M42) | Todas detectadas | [cierre E7 §6.2](etapa7-cierre.md) |
| E7-06 (réplica) | 4 mutaciones en una copia de `HEAD` | Las cuatro detectadas por alguna prueba. La de `Contributions::sum` +1 la ven 8 pruebas del oráculo de `assoc_rank`, y **no** la medición `assoc_measure` (R-3) | [cierre E7 §3.3](etapa7-cierre.md) |
| E8 (sondas) | 5 sondas en copia del árbol, dos motores | `10 passed; 0 failed`. Tres observaciones registradas | [cierre E8 §2.3](etapa8-cierre.md) |
| E9-02 | 28 mutaciones (M01 a M25, M27 a M29) | Todas detectadas | [etapa9-mutaciones-e9-02](etapa9-mutaciones-e9-02.md) |
| E9-03 (auditoría) | 5 mutaciones (M1 a M5) | Todas detectadas por la prueba objetivo | [etapa9-auditoria §3](etapa9-auditoria.md) |
| E9-15 (mutaciones de la Etapa 8) | M1 a M15 sobre `consolidation.rs` y D1 a D4 sobre `derived_measure.rs` | Las 19 pruebas tienen mutación. Dos mutaciones (D3 y D4) hacen fallar una sola prueba | [etapa9-mutaciones-e8](etapa9-mutaciones-e8.md) |
| E9-20 (medición E8-10) | 7 mutaciones sobre `derived_measure2.rs` | Todas detectadas | [etapa9-medicion §4](etapa9-medicion.md) |
| E9-22 (K1 y K2) | 1 mutación por hallazgo | Detectadas en los dos motores | [etapa9-k-e3-7 §8](etapa9-k-e3-7.md) |
| P1 (K1, formato v4) | 20 mutaciones (M1 a M20), todas en rojo (`exit=101`). M20 es la corrección de la revisión independiente | Ninguna sobrevivió. M15 muta una prueba, no `src/` | [post9-mutaciones](post9-mutaciones.md) |
| P2 (pruebas y texto) | 18 registros: Q1 a Q16, Q13b y Q13c. Todas las de pruebas nuevas o cambiadas, en rojo. **Q13b**, en verde, es un candidato descartado | Q13 muta la prueba, no `src/`. Q5 mueve la toma del bloqueo, y el bloqueo durante el marcado no tiene mutación que lo muestre | [post9-mutaciones](post9-mutaciones.md) (registro P2) |
| P3 (ADR-0016) | 17 mutaciones (M1 a M17), todas en rojo | Ninguna sobrevivió. M9 se repitió después de endurecer su prueba | [post9-mutaciones](post9-mutaciones.md) (registro P3) |

**Límites de las mutaciones.** Muchas se hicieron sobre una copia del árbol, una a una, y se restauraron. Varias son «amplias»: una mutación que rompe todo también hace fallar la prueba objetivo, y eso prueba poco (lo dice el propio documento de E9-15). El cierre de la Etapa 7 encontró que una mutación de `sum` no la veía la medición (R-3). La mutación de cada prueba nueva de E8-06 y E8-07 no tenía registro hasta E9-15 (F-10 de la Etapa 8).

### 3.7 Benchmarks y mediciones

Todas las cifras salen de los documentos citados. **Ninguna se midió en la máquina objetivo**, que el repositorio no nombra (C-09). Las condiciones de cada una están en la tabla.

| Medición (fecha) | Condiciones | Cifras principales | Qué no cubre | Fuente |
|---|---|---|---|---|
| **M0** (2026-10-27) | VM de 4 núcleos y 15,7 GiB, ext4, kernel 6.18. `fdatasync` de 4 KiB: p50 113 µs. Compilación en release. 410 ejecuciones, 0 fallos. Escalas small, medium y large. Tres repeticiones intercaladas | Cauce en `Sync`: LSM 3 087 pasos/s, árbol B+ 1 111. Historial simulado: LSM 66,6 mil revisiones/s, árbol B+ 8,3 mil | Máquina objetivo; carga de otros procesos no controlada | [m0-report §3](../testing/m0-report.md) |
| **Fase 1, T1 a T10** (2026-10-29) | Misma clase de VM. Carga de 1 minuto entre 0,97 y 1,90. `medium`, `Sync`, mediana de 3 repeticiones. 552 ejecuciones del barrido principal | T1: `agent_memory` p99 de escritura 3 938 µs, 2,78 veces la referencia LSM; T7: amplificación de `small_commits` 4,29 frente a 1,20 (rango 1,17 a 5,01 entre repeticiones); T4: cierre tras `bulk_load` 251 ms frente a 250 | Escala `large` incompleta (`range_scans` y `small_commits` no se ejecutaron en `large`). LazyFS, sin ejecutar. C-09 | [f1-report §5](../testing/f1-report.md) |
| **Etapa 3, U1 a U6** (2026-11-04) | VM KVM de 4 vCPU, disco virtual ext4 con el 90 % ocupado. Tres pasadas completas en máquina quieta, según el cierre | U1: 1,05 a 1,10 (cumple). U2 `history`: 1,50, 1,27, 0,95 (incumple en una pasada). U6 p99 de escritura con retención concurrente: 1,05 a 1,18; rendimiento 0,89 a 0,94 (incumple) | Sin índices ni relaciones en U6 (eso lo corrige E5-00) | [cierre E3 §4](etapa3-cierre.md), [etapa3-report](../testing/etapa3-report.md) |
| **U6 con índices y relaciones** (E5-00, 2026-11-06) | Salida cruda comprometida | p99 de escritura 1,05 a 1,18; rendimiento 0,89 a 0,94. Sigue incumpliendo. No bloquea | Sin nueva variación | [cierre E5 §5](etapa5-cierre.md), [u6 remedición](etapa5-u6-remedicion.md) |
| **Planificador** (E5-02/E5-03, 2026-11-06) | Máquina compartida con otros procesos («ruido de máquina compartida», según el cierre) | p99 +8,5 % a 100 000 unidades; +14,1 % a 20 000 en una ejecución y +0,3 % en otra. Observación `diagnostic` a 20 000: +5,9 % en una ejecución. Límites del acta: 10 % y 5 % | Máquina sin carga: pendiente | [cierre E5 §5](etapa5-cierre.md), [planificador §7](etapa5-planificador.md) |
| **Caché de resultados** (E6-04b, 2026-11-11) | Máquina de 4 CPU, carga ≈ 1, 20 000 unidades, cuatro corridas | p50 de W1 con caché: −97,8 % a −97,9 %. p99 de W1: +0,5, +7,8, +4,7 y +7,5 %. Medición emparejada del coste real: +3,4 a +4,9 % en la primera lectura tras escritura | Sin repeticiones, solo 20 000 y 100 000. Sin concurrencia. Sin variar el tamaño de la caché | [cierre E6 §4](etapa6-cierre.md), [medición §8](etapa6-medicion.md) |
| **Calidad y rendimiento asociativo** (E7-05, 2026-11-11) | Conjunto de 2 000 unidades y 64 consultas. Rendimiento a profundidad 2, doce corridas, máquina de 4 CPU con carga media de 1 a 9 | Calidad: orden simple nDCG@10 0,942; ranking 0,912; línea base 0,500. Índice p50 0,147 a 0,159 ms frente a referencia 103,9 a 110,5 ms | Otras escalas; las cifras absolutas no son comparables entre máquinas | [cierre E7 §4](etapa7-cierre.md), [medición](etapa7-medicion.md) |
| **Derivados, E8-07** (2026-11-12) | Conjunto nuevo de 100 consultas; 25 con vector de consulta, todas de una misma clase. P1 y P2 en doce corridas | Q1 = 1,000; Q2 = Q3 = 1,000 en el brazo con señal y sin ella (efecto techo). Q5 = 1,1 %. P1 (escritura con descriptor entre sin descriptor) = 1,165 a 1,238. P2 (búsqueda persistida entre en memoria) = 0,921 a 1,007. P3 y P4 se cumplen (286 de 286) | Q4 sin demostrar: tres clases sin vector. P2 no mide la lectura de los vectores persistidos (F-08) | [cierre E8 §0](etapa8-cierre.md), [medición E8](etapa8-medicion.md) |
| **Derivados, E8-10 en E9-20** (2026-11-13) | Una sola medición, con otra carga activa. Cuatro corridas de 12 rondas de 64 pares para P1 y P2. Un solo conjunto sintético de 2 000 unidades | Calidad: nDCG@10 con señal 0,9738; sin señal 0,4216; S1 +0,5522; Q5 1,6 %. P1 1,140 a 1,173; P2 0,997 a 1,001 | P1 y P2 **no** en reposo. Umbrales impresos, no afirmados (sección 5) | [medición E9](etapa9-medicion.md), [salida cruda](etapa9-medicion-salida.txt) |
| **SQLite, E9-17** (2026-11-12) | Escala tiny, 5 000 registros. 12 cargas, 5 repeticiones, durabilidades en pareja. 240 ejecuciones. Carga de 1 minuto entre 1,3 y 1,9 | Sin umbrales. Kerno es más lento en apertura, recuperación y cierre. Kerno es más rápido en lecturas puntuales, escaneos y escrituras, con la advertencia de que el modo `Buffered` no tiene garantías equivalentes | No se extrapola a otras máquinas ni a otras escalas | [informe SQLite E9](../testing/sqlite-report-e9.md) |

Los documentos de P1 a P3 no registran medición de rendimiento nueva. Un cambio de P3 sí afecta a una medida existente: el brazo con caché del banco `index_bench` (`crates/kerno-units/examples/index_bench.rs:335`) sube el observador a `Fingerprint`, así que su cifra incluye el coste de observar y no es comparable con la de la Etapa 6 ([post9-mutaciones](post9-mutaciones.md), P3).

---

## 4. Errores encontrados y corregidos

Esta lista recoge los defectos que los documentos registran como corregidos. Cada entrada dice cómo se encontró, cómo se corrigió y qué prueba lo guarda. Cuando el documento no nombra la prueba, lo dice.

### 4.1 Motor de almacenamiento (M0, 2026-10-27)

Estos defectos los encontraron el harness, las pruebas propias de cada prototipo y la revisión adversarial de M0. Son de los prototipos, no del motor actual, pero el motor actual nació de ellos ([m0-report §2.2](../testing/m0-report.md)).

| # | Motor | Defecto | Cómo se encontró | Corrección | Qué lo guarda |
|---|---|---|---|---|---|
| M0-1 | Árbol B+ | Se elegía el slot del meta por paridad de la secuencia. Tras perder un slot, el siguiente commit sobrescribía el único meta válido | Prueba unitaria propia | El informe lo registra como defecto encontrado durante la implementación; no dice cuándo se corrigió | Prueba unitaria propia |
| M0-2 | Árbol B+ | El directorio se sincronizaba solo si el archivo «acababa de crearse» | `single_io_failure_at_every_point` | Corregido | El barrido de fallos de un EIO |
| M0-3 | LSM | Si el `rename` del MANIFEST tenía éxito y el `fsync` del directorio fallaba, se borraban salidas que el MANIFEST podía referenciar | Revisión propia | Corregido | No la nombra el documento |
| M0-4 | Ambos | El directorio de la base nunca se sincronizaba en su padre: un corte podía hacer desaparecer una base nueva | Revisión adversarial | Corregido en ambos motores | Regresiones (no nombradas) |
| M0-5 | Árbol B+ (severidad alta) | La apertura tomaba como durable un meta que solo era visible. Sus páginas liberadas se reutilizaban, y un corte posterior podía destruir la base (~30 % de las simulaciones) | Revisión adversarial | `fsync` al abrir, con regresión | Regresión (no nombrada) |
| M0-6 | Árbol B+ (media) | Un bit dañado en el meta más reciente hacía volver en silencio al commit anterior | Revisión adversarial | Dos copias del meta por slot, con regresión | Regresión (no nombrada) |
| M0-7 | Árbol B+ (media) | Un pánico dentro del commit no envenenaba el handle | Revisión adversarial | Corregido | No nombrada |
| M0-8 | Árbol B+ (rendimiento) | Los valores de más de ~1 KiB no se cacheaban: un `pread` por página | Revisión adversarial | Caché unificada | No nombrada |
| M0-9 | LSM (media-alta) | Daño en medio del WAL más reciente tras un cierre sucio se tomaba como cola rota. Commits en `Sync` se perdían en silencio | Revisión adversarial | Bandera `SYNCED` por registro, con regresión | Regresión (no nombrada) |
| M0-10 | LSM (baja, rendimiento media) | `close()` abandonaba la compactación en curso | Revisión adversarial | Compacta al cerrar, con regresión | Regresión (no nombrada) |
| M0-11 | Árbol B+ y LSM (bajas) | Pánico con un archivo de crc válidos; desbordamiento de la secuencia en `u64::MAX`; lotes mayores de 4 GiB ilegibles; un snapshot borraba un SST huérfano; nombres no canónicos; aritmética sin comprobar | Revisión adversarial | Corregidos, con regresiones | No nombradas |
| M0-12 | Harness | Dos carreras con trabajo de fondo y dos huecos de cobertura (daño uniforme que casi nunca alcanzaba metas; desgarro solo a 512 B) | Los propios candidatos | Corregidos en el harness | Validado con mutaciones ([test-plan §2.1](../testing/test-plan.md)) |

### 4.2 Revisión adversarial de la Fase 1 (2026-10-31)

Fuente: [f1-adversarial-review §7](../testing/f1-adversarial-review.md). Todos estos se corrigieron antes de la fusión, salvo los que se marcan.

| # | Severidad | Defecto | Corrección | Qué lo guarda |
|---|---|---|---|---|
| **B1** | **Bloqueante** | Un fallo de escritura diferida de una SST se tomaba por durable: el error de `wait_writeback` se descartaba, y el `sync_data` final devolvía `Ok`. Un corte posterior podía perder datos confirmados. Revisión por lectura del código y de la semántica de Linux; no se reprodujo en un dispositivo real | `Sink::write_pending` propaga el error de `wait_writeback`, salvo `Unsupported` e `InvalidInput`. Commit `555565a`. Se comprobó en [`sst.rs:182`](../../crates/kerno-storage/src/sst.rs) | `write_amp_levers::a_failed_wait_for_writeback_never_loses_acknowledged_commits`. Verificado en esta nota: el nombre existe en [`write_amp_levers.rs`](../../crates/kerno-storage/tests/write_amp_levers.rs) |
| **G1** | Mayor (brecha de prueba) | El simulador modelaba el fallo de `WaitWriteback` como «sin efecto», lo contrario de Linux. Por eso ningún barrido podía ver B1 | `SimVfs` con `LinuxDropPages` descarta para siempre las escrituras cubiertas y marca el error como visto. Mismo commit `555565a` | La prueba de B1 (falla sin el arreglo) |
| **M1** | Menor | Una cabecera de WAL dañada con una sola trama se tomaba por «cabecera nunca sincronizada» y descartaba commits | El escritor sincroniza la cabecera antes de la primera trama. WAL-R6 trata toda trama con CRC válido bajo cabecera rota como `Corruption`. Commit `772f3fa` | Vectores `dirty-wal-one-frame` (positivo) y `wal-one-frame-broken-header` (negativo) |
| **M2** | Menor | `AccessMode::ReadOnly` abría `LOCK` con permiso de escritura: no podía abrir una base en un directorio de solo lectura | `StdVfs::lock` recurre a abrir `LOCK` solo para lectura, o el directorio. **Límite:** sin `LOCK`, el bloqueo recae en el directorio | `lock_works_in_a_directory_without_write_permission` en [`std_fs.rs`](../../crates/kerno-core/src/vfs/std_fs.rs) |
| **G3** | Menor (brecha de prueba) | El `kill -9` real solo tenía un escritor secuencial | Dos pruebas nuevas con cuatro hilos escritores (`process_kill.rs`), en `Sync` con `wal_preallocate` y en `Buffered`. **No cubren** `begin_write` ni `compact_range` | Las dos pruebas nuevas (sus nombres no aparecen en el documento) |
| **D1** | Menor (documentación) | Procedencia inexacta del registro de 1 000 semillas y una tabla rota | Corregido | No aplica |
| **G2** | Menor (brecha de prueba) | El cierre con subcompactaciones paralelas (valor por defecto) no tiene barrido de caídas | **Parcial:** se reescribió la frase de la matriz; el barrido no se añadió. **Abierto** | No aplica |

### 4.3 Etapa 3 (E3-10, 2026-11-04)

| # | Defecto o hueco | Cómo se encontró | Corrección | Qué lo guarda |
|---|---|---|---|---|
| D-1 | IU-2 incompleta en la lectura de la cabeza. Una cabeza `Archived` sobre una revisión `Create` se aceptaba. `get_unit` devolvía un ciclo de vida no confirmado | Revisión adversarial de E3-10 | `load_unit` compara `lifecycle_after(kind)`, la regla que usa `verify` | `unit_db::corruption_detected`, casos «lifecycle» y «metadata» |
| H-1 (huecos) | IU-1 solo se comprobaba en un escenario fijo | Mutación de la revisión | `policy_model` llama a `verify_immutable` en cada ronda | `policy_model` |
| H-3 (huecos) | Ningún control de que el contenido no entre en un error de API ni en un hallazgo de `verify` | Mutación de la revisión | Prueba nueva | `verify::sentinel_no_content` |
| PR #13 | `ChangeSet` vacío reportaba durabilidad sin cubrirla. `history_page` truncaba desplazamientos a `u32`. Una prueba de retención no esperaba su sincronización | Revisión del PR | Corregido en tres puntos | Prueba nueva en la retención (no nombrada) |

### 4.4 Etapa 4 (E4-05, 2026-11-05)

| # | Hueco | Corrección | Qué lo guarda |
|---|---|---|---|
| H-1 | El decodificador independiente no tenía vectores de `REL`, `REL_OUT` ni `REL_IN` | Se añadieron 10 vectores positivos y 25 negativos (`struct_vectors.rs`) | Los vectores `rel_*` |
| H-2 | `units-v1.md` no decía el orden de comprobación de la procedencia | Se corrigió el documento | No aplica |
| H-3 | Ninguna prueba dañaba la procedencia ya guardada | Prueba nueva | `relations::stored_provenance_corruption_is_detected` |
| H-4 | El workload diferencial de índices no tenía `restore` ni `run_retention` | Se añadieron | `random_step` de `tests/index.rs` |
| H-5 | Promoción formal de invariantes sin pruebas citadas | Se actualizó `invariants.md` | No aplica |

### 4.5 Etapas 6, 7 y 8

| Etapa | Defecto | Corrección | Qué lo guarda |
|---|---|---|---|
| 6 (E6-05, H-4) | El comentario de cabecera de `cache.rs` decía que la caché no estaba conectada al planificador | Se corrigió el comentario | No aplica (solo comentario) |
| 6 (E6-05, H-5) | El comentario decía «ninguna entrada más vieja que `max_age`», que no es cierto en reposo | Se corrigió el comentario. **El código no cambió** | No aplica |
| 7 (E7-00) | El conjunto congelado tuvo una corrección de un solo campo antes de cualquier código de recuperación (`6b2a3cb`). Las respuestas esperadas pasaron de 527 a 533 | Se documenta en [conjunto §9](etapa7-conjunto-evaluacion.md#9-corrección-previa-al-uso-fugas-de-un-solo-campo). Ningún umbral cambió | El `MANIFEST` del conjunto |
| 7 (R-11) | La mutación 1 de la medición no tenía su mensaje registrado | Se anotó el mensaje | No aplica |
| 8 (F-14) | El roadmap decía que E8-S0 seguía pendiente, aunque su informe estaba comprometido | Se corrigió el roadmap | No aplica |

### 4.6 Etapa 9

| Paquete | Defecto | Cómo se encontró | Corrección | Qué lo guarda |
|---|---|---|---|---|
| **E9A-03, H-1** (adelanto de la Etapa 9) | Un bloque de filtro de SST reescrito y resellado con un CRC válido pasaba la apertura. `get` respondía «no existe» para claves que la tabla contiene, sin error | Prueba de propiedades de apertura, E9A-03 | Apertura y `verify()` sin `deep` comprueban que el filtro contiene la última clave de cada bloque. Commit `d89c728` | `open_properties::resealed_zeroed_filter_is_refused_at_open_and_reported_by_verify` (verificado: existe). Límite residual en H-2 |
| **E9-02** (ADR-0005) | Un primer diseño de identificadores públicos tenía fallos que la revisión independiente encontró: valor de registro sin la cabecera común, falta de la marca de capa 3, borrado y exportación | Revisión independiente | Corregido el 2026-11-13 por delegación. Ver [units-v3](../file-format/units-v3.md) | `tests/public_ids.rs`, incluidas `tampered_text_is_refused_with_its_own_error` y `tampered_but_well_formed_tokens_name_nothing` |
| **E9-03, H-6 y H-8** | El detector de centinelas no buscaba fragmentos de 8 bytes. Un comentario decía «10^4 commits», y el código hace 4 000 o 6 000 | Cierre del adelanto | Detector con fragmentos; comentario corregido | `redaction_units.rs` y `redaction.rs`; detector con prueba propia |
| **E9-22, K1 (parcial)** | Desde el extremo que conserva su entrada de adyacencia, `query_relations` devolvía la respuesta de adyacencia como limpia cuando faltaba la entrada espejo | Lectura de la revisión de K-E3-7 y prueba de daño | `execute_relations` comprueba la entrada espejo de cada relación (`all_mirrored`). Si falta, `Fallback` `Invalid(Unmirrored)` | `one_adjacency_entry_missing_{small,large}_store` en `relations.rs`, dos motores |
| **E9-22, K2 (parcial)** | Con el conteo de `IDX_META` intacto, la ruta `IDX_PRIMARY` devolvía menos unidades sin error | Prueba de daño | `scan_entries` cuenta lo que recorre y da `Corruption` si no coincide con el conteo | `index_path_with_a_missing_entry` en `index.rs`, dos motores |
| **E9-22, K3** | Sospecha de que `get_unit` no validaba `oldest_rev` | Lectura de la revisión | **No se reprodujo**: `UnitHead::decode` ya lo exige | `get_unit_checks_oldest_rev` en `relations.rs`, dos motores (se mantiene como guarda) |
| **H-8 a H-12** (adelanto y cierre) | Comentarios, documentos y una cita de línea desactualizados | Revisión del adelanto | H-8 se cerró en E9-03; H-9 y H-12 en el cierre de la Etapa 9 (§8); H-10 y H-11 se marcaron resueltos en el cierre del adelanto | No aplica |
| **Erratas del cierre E9 (§7)** | Tipo de `DerivedDb::close` (devuelve `Result`, no `()`); línea de `compact_range` que cita ADR-0009 (`db.rs:2009`; al escribir el acta era `db.rs:2011`); cita del fixture de ADR-0020 | Revisión del cierre | Corregidas en el cierre, sin editar los ADR aceptados | No aplica |

Los hallazgos F-02, F-03 y F-04 de la Etapa 8 no aparecen aquí como corregidos. El cierre de la Etapa 9 solo los asigna a E9-10 y E9-11 y no dice si se resolvieron. Están en la sección 5.4.

### 4.7 Posteriores a la Etapa 9: P1, K1 y la revisión independiente

| Hallazgo | Cómo se encontró | Corrección | Qué lo guarda |
|---|---|---|---|
| **K1**, relaciones con registro de conteo. Una entrada de adyacencia perdida no dejaba huella: desde su extremo, `relations_of` devolvía una respuesta corta sin error, y `query_relations` no daba `Fallback` (sección 5.1) | Revisión de K-E3-7, en la Etapa 9 ([etapa9-k-e3-7](etapa9-k-e3-7.md) §9) | Formato de unidades v4 (`0c2c3d5`): un registro de conteo por `(unidad, dirección, tipo)`. El escritor lo suma y lo resta. El lector lo compara con lo que recorre. `verify_units` lo comprueba con UNIT-45 y UNIT-46 ([units-v4](../file-format/units-v4.md)) | `rel_counts.rs`, en los dos motores: 30 pruebas que corren y 1 `#[ignore]` que regenera el fixture. 19 mutaciones de P1, todas detectadas. Límites: bases de antes de la capa 4 y doble daño (sección 5.1) |
| **Defecto de la revisión independiente de P1.** Una creación de relación rechazada dejaba un contador huérfano. `create_relation` ajustaba los dos contadores antes de que `record.encode()` validara la procedencia. Si el `encode` rechazaba la relación, el contador quedaba en el change set abierto y un `commit` posterior lo escribía. Entonces `relations_of` daba `Corruption` y `verify_units` UNIT-45, sobre un almacén sin daño | Revisión independiente de P1 (registro de mutaciones, «Corrección posterior de K1») | Construir y codificar todas las escrituras antes de tocar los contadores (`crates/kerno-units/src/db.rs:1473-1487`). `delete_relation` hace lo mismo con sus claves (`db.rs:1503-1509`). La importación se reordenó igual (`import.rs:213`, commit `0310bb3`). Commit de la creación: `4b65c51` | `a_refused_create_leaves_no_count` en `rel_counts.rs`, en los dos motores. Antes de la corrección daba `Corruption` en `relations_of` en ambos motores. Mutación **M20** (los dos `adjust_count` movidos antes de la codificación): roja en los dos motores (`exit=101`). El caso de la importación no admite prueba, porque su change set es privado y se abandona sin `commit` ante un error |

La revisión independiente de P1 sigue dada como pendiente en [post9-pendientes](post9-pendientes.md) (punto 2, desglose). Este informe registra el defecto que halló y su corrección, según [post9-mutaciones](post9-mutaciones.md).

### 4.8 Posteriores a la Etapa 9: P2, pruebas, mensaje y prueba intermitente

| Hallazgo | Corrección | Qué lo guarda |
|---|---|---|
| **Pruebas que faltaban** (cierre de la Etapa 9, puntos 4, 5, 6 y 10). Faltaban: el orden «bloqueo del registro, commit» de los derivados (F-09); la prueba de que los errores y las observaciones no llevan contenido; la prueba de que `compact()` rechaza un handle envenenado y no amplía su rango (E9-08g); y el registro de mutaciones de la Etapa 8 (E9-15) | Pruebas nuevas en `derived_lock_order.rs` (dos motores, con el gancho `hook_registry_locked`, solo con `test-support`), en `reclaim.rs` y `db.rs` para `compact()`, y aserciones de cobertura en `redaction_units.rs` | Registros Q1 a Q13 de [post9-mutaciones](post9-mutaciones.md) (registro P2). Límites: no se prueba que el bloqueo siga tomado durante el marcado; los fragmentos de menos de 8 bytes siguen sin buscarse (cierre E9, punto 5) |
| **Mensaje de `NoValidSources`** (punto 3). El texto no decía qué dato faltaba | Ahora dice: «no source of derived datum <id> is valid any more» (`crates/kerno-units/src/derived_stale.rs:124`). Cambiaron las dos pruebas que fijaban el texto (`consolidation.rs`, `derived_stale.rs`) | Mutación Q7. Commit `2356ff3` |
| **Prueba intermitente** `m0_profile::shared_range::space_is_reclaimed_after_range_deletes` (punto 11). Su causa fue una suposición de temporización de la propia prueba, no un defecto del motor. Medía el tamaño del directorio justo después de la última escritura de cada recarga, sin esperar a que la compactación de fondo alcanzara al escritor. Con la máquina cargada, el único hilo de compactación podía quedar un ciclo atrás. Los cuatro fallos fueron de la aserción de tamaño tras recargar, nunca de la de claves | Función `settled_size` (`crates/kerno-testkit/src/harness/mod.rs:72`): si el tamaño supera la cota, vuelve a medir cada 5 ms, hasta 30 s. La cota, el número de ciclos y las comprobaciones no cambian. Con carga, 4 fallos en 600 ejecuciones antes del cambio y 0 después. Sola, 0 en 200 antes y después. La prueba hermana `functional::space_is_reclaimed` tenía la misma suposición: 1 fallo en 300 antes y 0 después, y se corrigió en el mismo sitio | Mutaciones Q14 a Q16. Límites: el primer llenado no espera; un motor que no reclama falla 30 s más tarde; 600 ejecuciones sin fallo no prueban que la prueba no pueda fallar |

### 4.9 Posteriores a la Etapa 9: P3, ADR-0016 y la revisión de D16

| Carencia | Estado antes | Corrección | Qué lo guarda |
|---|---|---|---|
| **Trabajo abierto 1.** La explicación del plan no decía si se consultó la caché, ni si respondía estado aprendido | `explain_*` no lo decía | `Plan::cache()` (`crates/kerno-units/src/plan.rs:328`) devuelve un `CacheView`: `Off`, `Miss`, `Invalidated`, `Discrepancy`, `Hit { evidence, cost_ns }` o `NotApplicable`. Lo calcula `ResultCache::peek`, que no deja registro, no cuenta y no elimina nada. `Plan::answering_path()` (`plan.rs:335`) dice si responde el estado aprendido | `the_plan_says_whether_the_cache_was_consulted_and_whether_learned_state_answers` y `the_plan_does_not_announce_an_entry_that_fails_its_check` (`cache_planner.rs`, dos motores). Mutaciones M9, M13 a M16 |
| **Trabajo abierto 2 (H-3 de la Etapa 6).** La evidencia de creación era constante: `Evidence { observations: 1, window: Duration::ZERO }` | Evidencia fija | La evidencia es la del observador (`Observer::evidence_of`, `crates/kerno-units/src/observation.rs:424`). `CacheEntry::build` rechaza una entrada sin observaciones (`NoEvidence`) o sin coste (`NoCost`) (`cache.rs:351-354`). **Cambio de comportamiento:** el observador está apagado por defecto. Por debajo de `Fingerprint` no guarda registros por ejecución, así que con la caché encendida el planificador no crea entradas y ninguna lectura se sirve de la caché. Para usarla hay que subir el observador a `Fingerprint` o más | `relation_evidence_comes_from_the_observer` y `observations_never_act_as_authority` (`cache_planner.rs`). Mutaciones M1 a M6 y M17 |
| **Trabajo abierto 2, coste.** Las entradas no llevaban coste | Sin coste | `CacheEntry::cost_ns` (`cache.rs:376`): nanosegundos de la ejecución que la entrada sustituye. `Transition::Created { evidence, cost_ns }` y `CacheView::Hit { cost_ns }`. Límite: una sola medida por entrada, no un modelo de coste | `the_created_record_carries_the_evidence_and_the_measured_cost` (`cache.rs`). Mutaciones M6 y M8 |
| **Trabajo abierto 3.** `set_key` borraba entradas, registro y contadores sin dejar huella | Sin registro | `Transition::KeyReplaced { entries }` en el registro nuevo (`cache.rs:559`). `clear` sigue dejando `Cleared` | `replacing_the_key_leaves_a_record_of_the_wipe` (`cache_planner.rs`, dos motores) y `set_key_leaves_one_record_of_the_wipe_in_the_new_log` (`cache.rs`). Mutaciones M11 y M12 |
| **Revisión de D16, punto 1 (G1 y G5), antes de P3.** `ResultCache::create` y `activate` eran públicas, y `CacheEntry::new` y `Dependencies::current` también: un llamador podía plantar filas que luego se servían | El planificador usa versiones `pub(crate)` (`store`, `arm`, `build`, `in_force`). `create` y `activate` tienen `cfg(any(test, feature = "test-support"))` (`cache.rs:907-914`), y ADR-0016 dice lo mismo de las otras dos | Cuatro doctests `compile_fail`, uno por función, según ADR-0016, punto 1 | La descripción está en [ADR-0016, «Revisión de 2026-11-13», punto 1](../adr/0016-adaptacion-e-invalidacion.md) |

**Fuera de los paquetes.** En la Etapa 4, `kerno::retention_every_point_paced_power_loss_sync` falló una vez en seis ejecuciones (M-11). No se corrigió. No se reprodujo en E5-05 ([cierre E5 §5](etapa5-cierre.md)).

---

## 5. Errores conocidos y abiertos

Esta sección recoge cada defecto o límite abierto que los documentos registran. Para cada uno dice el impacto, cómo reproducirlo u observarlo cuando el documento lo dice, y qué soluciones investigaron los documentos, con sus compromisos. Donde un documento recomienda una solución, se marca. **Donde no hay recomendación, se dice.**

### 5.1 Relaciones e índices

| Id | Qué pasa | Impacto | Cómo reproducir u observar | Soluciones investigadas (compromisos) | Recomendada |
|---|---|---|---|---|---|
| **K1**, límite que queda (cerrado para relaciones con registro de conteo en P1; el defecto se da por corregido en la sección 4.7) | Una relación de una base **de antes de la capa 4** (sin conteo) tiene una entrada de adyacencia perdida: desde ese extremo `relations_of` devuelve una respuesta corta, sin `Fallback`. Solo `verify_units` (UNIT-36) lo ve | Respuesta incompleta sin error en bases antiguas | Prueba `one_adjacency_entry_missing_{small,large}_store` en `relations.rs`: borra a mano `REL_IN` o `REL_OUT` y compara. Base antigua: fixture `db-legacy-relations` ([units-v4 §8](../file-format/units-v4.md)) | (a) Recorrer todas las relaciones en una migración: costoso, y abrir no escribe (I-18). (b) Un registro de conteo que se mantiene al escribir, que es lo que hace P1: sin migración; las bases antiguas no tienen conteo hasta que se toca cada `(unidad, dirección, tipo)`. (c) Recorrer `REL` en cada consulta: más de una lectura por relación devuelta | **Ninguna migración recomendada.** P1 queda como límite declarado ([units-v4 §7](../file-format/units-v4.md)) |
| **K1**, doble daño (límite de P1) | Una entrada perdida **junto con** su registro de conteo deja una lectura coherente consigo misma | Igual que K1, sin detección en consulta | Descrito en [units-v4 §7](../file-format/units-v4.md). `verify_units` lo reporta vía UNIT-36 | Ninguna investigada | No aplica |
| **K2** (restante) | Con el conteo de `IDX_META` rebajado para coincidir con la entrada perdida, la ruta de índice responde menos sin error. La ruta `IDX_TYPE` no tiene conteo por tipo, así que una entrada `IDX_TYPE` perdida da respuesta corta | Respuesta incompleta sin error | Prueba `index_path_with_a_missing_entry` en `index.rs`. Lo que no ve: conteo rebajado | No investigada en los documentos. P1 aplica a relaciones el mismo principio de registro de conteo | No hay recomendación |
| **K4** | `change_set_info` no cruza los miembros de un `ChangeSet` con su `REV_META` | Menor: un historial dañado puede no verse en esa lectura | Inferencial: se lee del código. Prueba: UNIT-33 en `verify_units` | Comprobar cada miembro: leer el `REV_META` de cada uno | No hay recomendación. El cierre de la Etapa 9 lo deja abierto y sin cambio |
| **K5** | `get_relation` no comprueba `created_seq` ni los extremos | Menor | Inferencial. UNIT-36 lo reporta | Leer las dos cabeceras de los extremos y las dos entradas (dos lecturas más por consulta) | No hay recomendación |
| **H-1 de la Etapa 7** (R-2) | Con 64 relaciones o más y una relación sin entrada en ninguno de sus extremos, la respuesta por índice difiere de la referencia, sin fallback. P1 lo cubre solo si la relación tiene conteo | Respuesta incompleta sin error para bases sin conteo | `tests/assoc_indexed.rs` (prueba que fija el límite, antes `completeness_is_not_a_query_time_check`) | Conteo por `(unidad, dirección, tipo)`: P1 lo implementa para relaciones con conteo. Chequeo en consulta: caro | Conteo: implementado en P1 para relaciones con registro de conteo. Las bases sin conteo siguen abiertas |
| **H-1 de la Etapa 5** (completitud) | `check_units` y `check_relations` comprueban que la respuesta de un camino de índice sea subconjunto correcto, no que sea completa | Una respuesta incompleta de índice no se ve en consulta | Lectura de `planner.rs`. Lo ven la prueba diferencial y `verify_units` (UNIT-37) | Ninguna investigada | Aceptada como limitación |
| **H-2 de la Etapa 5** | El fallback con índice `Building` no se ejerce a través de `query`, porque una instantánea no ve ese estado | Cobertura parcial de un estado transitorio | `candidates_carry_the_usability_the_state_gives` cubre el plan, no la consulta | No investigada | Aceptada como salvedad |
| **M-10 de la Etapa 4** | Una reconstrucción concurrente que falla puede escribir `Failed` sobre un `Ready` que acaba de escribir otra | Conservador: el índice se declara no utilizable cuando lo era | Inferencial | Ninguna. Una reconstrucción lo repara | Aceptada |
| **M-7 de la Etapa 4** | La consulta solo por tipo tiene p95 de 17 ms con 100 000 unidades, frente a un criterio de 10 ms | Rendimiento de W1 fuera de criterio | Medido en [etapa4-workloads](etapa4-workloads.md) | La cota ideal (abrir los 999 resultados) ya cuesta 12 ms: ningún índice baja de ahí | Aceptada |

### 5.2 Rendimiento

| Id | Qué pasa | Impacto | Cómo observar | Soluciones investigadas (compromisos) | Recomendada |
|---|---|---|---|---|---|
| **K-E3-8 (U2)** | `history` llega a 1,50 veces la referencia en una pasada de tres | Objetivo U2 no cumplido | [etapa3-report §8–§9](../testing/etapa3-report.md) | (a) Juzgar U2 sobre `history_page`, que da 0,58 a 0,90 veces: cambia la definición del criterio. (b) Abaratar la materialización de `Revision` | No hay recomendación explícita. El propietario lo integró como riesgo (2026-11-04) |
| **K-E3-9 (U6)** | Con retención concurrente, p99 de escritura 1,05 a 1,18 veces y rendimiento 0,89 a 0,94 | Objetivo U6 no cumplido. Causa: rotaciones, vaciados y compactaciones de fondo | [etapa3-report](../testing/etapa3-report.md) y [u6 remedición](etapa5-u6-remedicion.md) | Limitar el ritmo de ese trabajo de fondo en el motor | **Sí:** el cierre de la Etapa 3 lo recomienda |
| **T1 (Fase 1)** | p99 de escritura de `agent_memory`: 2,78 veces la referencia LSM, frente a un máximo de 2,0. La mejora frente a M0 es real | Objetivo no cumplido | [f1-report §5.2](../testing/f1-report.md) | Revisión de contingencia de ADR-0001, que es decisión del propietario | No hay recomendación en el documento. La decisión está abierta |
| **T2 y T3 (Fase 1)** | p99 de commit de Cauce 1,95 veces la referencia (objetivo 1,2); p99 de LiveSpec 1,79 veces el árbol B+ (objetivo 1,5) | Objetivos no cumplidos | [f1-report §5.3](../testing/f1-report.md) | La revisión de la condición 5 | No hay recomendación en el documento |
| **T4 (Fase 1)** | Cierre tras `bulk_load` `medium`: 251 ms frente a 250 ms. Rango de 235 a 296 ms | Un fallo de 1 ms en el ruido: se registra como fallo | [f1-report §5.2](../testing/f1-report.md) | Ninguna | No aplica |
| **T5 (Fase 1)** | Por debajo de 0,9 veces la referencia LSM en `bulk_load` (0,85), `point_reads` (0,80), `range_scans` (0,71) y `cauce` (0,87) | Objetivo no cumplido en cinco cargas | [f1-report §5.3](../testing/f1-report.md) | Ninguna | No aplica |
| **T7 (Fase 1)** | Amplificación de escritura de `small_commits`: 4,29, con rango de 1,17 a 5,01 | Objetivo no cumplido, y inestable | [f1-report §5.3](../testing/f1-report.md) | El documento dice que depende de si el trabajo de compactación cae en la ventana medida. Sin solución | No hay recomendación |
| **Planificador (R-E5-1)** | Coste p99 +8,5 % a 100 000 unidades; +14,1 % a 20 000 en una ejecución (la segunda da +0,3 %) | Riesgo de rendimiento, no bloqueante | [cierre E5 §5](etapa5-cierre.md) | Repetir sobre máquina sin carga | **Sí:** el cierre lo propone, y no se ha hecho |
| **Observación `diagnostic`** | +5,9 % a 20 000 unidades en una ejecución; límite del acta, 5 % | Riesgo de rendimiento, no bloqueante | [cierre E5 §5](etapa5-cierre.md) | Repetir sobre máquina sin carga | **Sí**, el mismo |
| **Caché, p99 de W1** (E6) | p99 de W1 entre +0,5 y +7,8 % en cuatro corridas. La primera lectura tras escritura no se puede servir desde la caché. Mejora neta **no afirmada** | La caché queda apagada por defecto | [cierre E6 §4](etapa6-cierre.md) | (a) Repetir con más rondas. (b) Evitar copiar el resultado al guardarlo: cambia el tipo público `CachedResult` y `QueryOutcome` | No hay recomendación. El propietario eligió mantenerla apagada |

### 5.3 Calidad de recuperación y ranking

| Id | Qué pasa | Impacto | Cómo observar | Soluciones investigadas (compromisos) | Recomendada |
|---|---|---|---|---|---|
| **Q4 (Etapa 8)** | Solo una de las cuatro clases de consulta tiene vectores de consulta en el conjunto de E8-07 (25 de 25). Las otras tres tienen 0 | El criterio de calidad con y sin señal no se pudo calcular en tres clases | [cierre E8 §3](etapa8-cierre.md) | Un segundo conjunto de evaluación (E8-10), con vectores en cada clase, congelado antes de medir. Se midió una vez en E9-19 y E9-20, con carga. Q4 la sustituyen S1 y S2 ([acta §5](etapa9-acta.md)) | **Hecho con límites:** S1 y S2 sustituyen a Q4 ([requisitos-1.0](requisitos-1.0.md), punto 3). Los umbrales de calidad se imprimen y no se afirman |
| **Efecto techo (Q2, Q3, Q4)** | 1,000 en ambos brazos en las 25 consultas con vector | Las 25 consultas no distinguen los brazos | [cierre E8 §3](etapa8-cierre.md) | El mismo conjunto de E8-10 | Igual que Q4 |
| **T4 (Etapa 7)** | Exige 1,036 con un orden simple de 0,942. Un nDCG no pasa de 1 | Un criterio inalcanzable tal como está escrito | [cierre E7 §3.2 R-7](etapa7-cierre.md) | (a) Reescribir T4 antes de medir de nuevo. (b) No tocarlo. El propietario decidió no reescribirlo ahora | Decisión del propietario: no reescribir (2026-11-12) |
| **Ranking (R-5, R-6)** | Las unidades sin nombrar valen 0, lo que penaliza al ranking. El resultado por clase cambia con el desempate del orden simple | Conclusiones por clase frágiles | [cierre E7 §3.4](etapa7-cierre.md), controles exploratorios | Medir con otro conjunto | No aplica |
| **R-8 y R-9 (Etapa 7)** | Configuración medida sin señal de tipo ni vectores. Los pesos se justificaron antes de medir, no se midieron por separado | El ranking queda sin evaluar en su configuración completa | [cierre E7 §3.2](etapa7-cierre.md) | Otra medición con otro conjunto | No aplica |
| **R-10 (Etapa 7)** | La línea base B1 no recibe las restricciones de la consulta. La comparación con B1 favorece a las consultas asociativas por diseño | Comparación con B1 sesgada | [cierre E7 §3.2](etapa7-cierre.md) | Ninguna investigada | No aplica |
| **R-3 (Etapa 7)** | La comprobación de I-41 en la medición no es independiente de `Contributions::sum`. Solo el oráculo de `assoc_rank` lo ve | La medición no es evidencia independiente de I-41 | [cierre E7 §3.3](etapa7-cierre.md), mutación M-R4 | Ninguna investigada | No aplica |
| **R-4 (Etapa 7)** | El contador de C1 es tautológico: suma el mismo número dos veces | La cifra 757 de 757 no es un recuento independiente | [cierre E7 §3.2](etapa7-cierre.md) | Ninguna investigada | No aplica |
| **Ranking en la ruta por defecto** | El ranking no es el orden por defecto. Está apagado (`RankMode::Off`) | El orden que sirve la consulta es «saltos, luego identificador» | [cierre E7 §8](etapa7-cierre.md) | Decisión del propietario, 2026-11-12 | No aplica |
| **F-08 (Etapa 8)** | P2 no mide el coste de leer vectores persistidos: compara dos búsquedas sobre el mismo conjunto ya cargado | Un coste real no medido | [cierre E8 §3](etapa8-cierre.md) | Un diseño de banco distinto. Una lectura única de 20 000 vectores tardó 9,8 a 11 ms y se informa aparte | No hay recomendación |

### 5.4 Derivados, resúmenes y supresión

| Id | Qué pasa | Impacto | Cómo observar | Soluciones investigadas (compromisos) | Recomendada |
|---|---|---|---|---|---|
| **F-02** | Tras reconstruir un resumen, construir de nuevo sobre las mismas fuentes devuelve `Duplicate` con un mensaje poco claro | Ergonomía | Sonda P3 en la [sección 2.3 del cierre E8](etapa8-cierre.md) | Quien quiera el resumen debe guardar el id nuevo, que es lo que el código ya permite. El cierre de la Etapa 8 lo recoge como ergonomía dentro del grupo D de la lista de fricción | No hay recomendación |
| **F-03** | `read_input` devuelve `None` si el head no tiene carga útil, y `valid_inputs` omite la fuente sin error | Una fuente que falte se omite en silencio en `rebuild` y en `supersede`. Solo una unidad borrada lo produce, y el camino no se alcanza | Lectura de `derived_stale.rs`: `read_input` y `valid_inputs` (líneas 385 y 406 en el árbol actual; el cierre E8 las cita en 354 a 372). Asignado a E9-11; el cierre de la Etapa 9 no dice si se corrigió | Ninguna investigada | No aplica |
| **F-04** | La identidad de un derivado incluye el perfil, pero ninguna prueba cambia solo el perfil y comprueba que la identidad cambia | La propiedad se cumple en el código y no tiene prueba que la muestre | Búsqueda de `profile` en `tests/` | Añadir la prueba | No hay recomendación |
| **F-05** | Los resúmenes viven solo en memoria. No hay medición de centinelas sobre ellos | Sin bytes en disco que medir | Cabecera de [`derived_suppression.rs`](../../crates/kerno-units/tests/derived_suppression.rs) | Ninguna: los resúmenes no tienen formato en disco (D8) | No aplica |
| **F-09** | El orden «bloqueo del registro, commit, marca» del commit guardado no tiene prueba | Un bloqueo tardío posterior al commit sigue en verde en la prueba de estrés (cierre E9, punto 4) | La nota del propio código de `derived_stale.rs` (el commit está en la línea 549 del árbol actual; el cierre E8 cita las líneas 495 a 502) | Mostrarlo exige un lector que compita con el commit, y no se puede hacer determinista | No hay recomendación |
| **F-10** | Falta la mutación registrada de cada prueba nueva de E8-06 y E8-07 | Brecha de proceso | Cierre E8, punto 4 de la acta | Cerrada en E9-15, con dos mutaciones que hacen fallar una sola prueba | Cubierta, con salvedades |
| **F-11** | La comprobación T5 de ADR-0020 (construir el binario de `d01f646` y abrir el fixture) no se ejecutó | El fixture de ADR-0020 no tiene una comprobación ejecutada | Cierre E8 §2.4 | Un binario de referencia que no se construyó | No hay recomendación |
| **F-12** | No hay registro de las rondas de revisión de E8-03 y E8-04. Se reconstruyen a partir de los mensajes de commit | La cuenta de rondas no está verificada: el cierre cita cinco, y el historial muestra seis commits con revisión visible | [cierre E8 §7.1](etapa8-cierre.md) | Ninguna | No aplica |
| **F-13** | Quince puntos de fricción en la API pública; uno toca una promesa (`forget` es solo lógico) | Bloquea 1.0 como trabajo previo | [FRICTION](../../examples/agent-memory/FRICTION.md). La prueba `forgotten_text_stays_in_the_files_after_retention` encuentra el texto en `000003.sst` | Orden propuesto: primero el grupo F (la promesa), luego B y A, C y D después, E al final | Orden del cierre E8 §6 |
| **Compactación inalcanzable** | `UnitDb::compact` necesita `TestHooks`, y `KernoDb::compact_range` es un método de prueba: la envoltura pública no los expone | El borrado físico no es alcanzable desde la API pública | [cierre E8 §5, punto 9](etapa8-cierre.md), grupo F | Exponer la compactación en la API pública | No decidido |
| **Limitación de E9-21, `set_policy`** | `set_policy` no acepta `KeepLast(1)`. El ejemplo poda de forma explícita | El ejemplo no usa la política que pide su acta | [cierre E9, punto 8](etapa9-cierre.md) | Ninguna investigada | No aplica |
| **E9-12 y F-09, parcial** | P2 prueba el orden «bloqueo del registro, commit» (`derived_lock_order.rs`, dos motores). No prueba que el bloqueo siga tomado durante el marcado: ninguna observación externa lo ve sin un lector que compita | Igual que F-09 | [cierre E9, punto 4](etapa9-cierre.md), [post9-pendientes](post9-pendientes.md) (punto 4) | Ninguna | No aplica |
| **Vectores deterministas** | Los vectores son deterministas con semilla. No dicen nada de la calidad de un modelo real. No se decidió un modelo (D5 (b) de la Etapa 8) | La calidad semántica no se ha medido | [cierre E8 §5, punto 4](etapa8-cierre.md) | Un conjunto con vectores de un modelo real. Ninguno existe | No hay recomendación |

### 5.5 Caché, planificador y observabilidad

| Id | Qué pasa | Impacto | Cómo observar | Soluciones investigadas (compromisos) | Recomendada |
|---|---|---|---|---|---|
| **H-5 de la Etapa 6** | Una entrada más vieja que `max_age` no se sirve nunca, pero sigue en memoria mientras la caché no cree una entrada nueva | La cota por edad no se cumple en reposo. La memoria sí está acotada por tamaño | `grep -n prune_entries crates/kerno-units/src/cache.rs` | Llamar a `prune_entries` también en `stats()` y `records()`: una línea por función, con una prueba | No hay recomendación |
| **H-6 de la Etapa 6** | Las causas de la caché (`precondition_failed`, `invalidated`, `disabled`, `discrepancy`) no llegan a `ExecutionObservation` | Un fallback de la caché no aparece en la observación | `observation.rs` y `planner.rs` | Llevar la causa a `ExecutionObservation` (cambio de `FallbackCause`), o dejarla en el registro de transiciones | Aceptada por delegación como riesgo registrado (cierre E6 §7) |
| **H-7 de la Etapa 6** | La mutación «con la caché apagada, sigue el camino de la caché» sobrevive a toda la suite de caché | I-36 está verificada en resultados y contadores, no en coste | Mutación de `if !self.cache.enabled()` (el cierre E6 la cita en `planner.rs:354`; en el árbol actual está en las líneas 470 y 636 de `planner.rs`; sección 3.3 del cierre E6) | Un punto de inyección para observar la lectura del índice | Aceptada por delegación como riesgo |
| **H-1 de la Etapa 6** | `check_units` y `check_relations` comprueban subconjunto, no completitud | Igual que H-1 de la Etapa 5 | Lectura | Prueba diferencial | Aceptada como limitación |
| **H-4 de la Etapa 5** | La retención por edad comprueba solo el frente de la cola. Con un reloj que retrocede, un registro viejo puede quedar detrás | Un registro viejo puede retenerse de más | `observation.rs`, nota `ponytail:` | Recorrer la cola al leer | Aceptada, con su ruta documentada |
| **Pérdida de observaciones (H-3 de la Etapa 5)** | Un registro se pierde y se cuenta cuando el cerrojo está ocupado | Observación incompleta por diseño | [observación §3](etapa5-observacion.md) | No esperar | Diseño aceptado |

### 5.6 Storage, durabilidad y pruebas

| Id | Qué pasa | Impacto | Cómo observar | Soluciones investigadas (compromisos) | Recomendada |
|---|---|---|---|---|---|
| **LazyFS sin ejecutar** | El escenario A sobre un dispositivo que pierde páginas no tiene ninguna ejecución registrada | El comportamiento ante pérdida de páginas en un dispositivo real no está demostrado | `run-lazyfs.sh` devuelve 2 y no se pone en verde sin FUSE. Workflow manual y semanal | Ejecutar `lazyfs.yml` en la CI, o localmente con FUSE | **Sí:** es el pendiente de WP14 |
| **Límite de `LOCK` (M2)** | Sin `LOCK`, el bloqueo recae en el directorio y no excluye a un escritor que cree `LOCK` después | Dos escritores podrían coexistir si uno crea `LOCK` tarde | Marca `ponytail:` en el código de `std_fs.rs` | Ninguna investigada | Aceptado |
| **G2 (parcial)** | El cierre con subcompactaciones paralelas (valor por defecto) no tiene barrido de caídas | Una caída durante un cierre con subcompactaciones no se prueba | Matriz de fallos, fila SC | Añadir un barrido con `Background::Threads` | Abierto, no bloqueante |
| **G3 (parcial)** | El `kill -9` no cubre `begin_write` ni `compact_range` | Dos caminos sin muerte real de proceso | [f1-adversarial-review §7](../testing/f1-adversarial-review.md) | Añadir las pruebas | Abierto |
| **Hallazgos de rendimiento de M0 no verificados** | Contención de contadores globales en lecturas concurrentes (LSM); `fdatasync` de un WAL que crece sin preasignación (LSM); copias de páginas en lotes muy grandes (árbol B+). No se verificaron por falta de cuota de agentes | Riesgo de rendimiento sin medir | [m0-report §2.2](../testing/m0-report.md) | Medir | No hay recomendación |
| **C-4 de ADR-0009 (H-4 del adelanto)** | La cláusula de conteo de C-4 no se sostiene: `pinned_files = 3` mientras solo `000007.sst` contiene el valor y la clave | La cláusula del ADR no coincide con la medición | Salida de §2.1 de [etapa9-adelanto-cierre](etapa9-adelanto-cierre.md) | Ninguna. El ADR es aceptado y no se edita. D17 lo dejó registrado | Contradicción registrada (D17) |
| **H-5 del adelanto de la Etapa 9 (no medido)** | Bloques liberados del medio (C-5), memoria, cachés, lápidas de punto, borrado por hilo de fondo y handle de larga vida no se miden | El borrado y la memoria no están verificados | [suppression_survival.rs](../../crates/kerno-storage/tests/suppression_survival.rs), cabecera «Not exercised» | Medir cada uno | No aplica |
| **Prueba intermitente de la Etapa 4 (M-11)** | `retention_every_point_paced_power_loss_sync` falló 1 vez en 6 ejecuciones | Igual que la anterior | [cierre E4 §4](etapa4-cierre.md) | Ninguna. El cierre dice que no hay una corrección pequeña y clara | Aceptada como registrada |

### 5.7 Seguridad y privacidad

| Id | Qué pasa | Impacto | Cómo observar | Soluciones investigadas (compromisos) | Recomendada |
|---|---|---|---|---|---|
| **T-03** | Acceso de lectura al disco o a los backups: sin cifrado en reposo | Un disco robado da los datos | [threat model T-03](../security/threat-model.md) | (a) AEAD por bloque (ADR-0006, opción 1), con un `KeyProvider` fuera del directorio: cambia el formato y usa la ranura reservada de 40 bytes. (b) Cifrado a nivel de sistema de archivos o dispositivo, delegado al sistema operativo (ADR-0006, opción 2): sin cambio de formato, pero sin protección de copias exportadas ni control de claves por Kerno. (c) Cifrado por registro o campo (ADR-0006, opción 3): permite borrado criptográfico selectivo, con más metadatos | Opción (a) como **diseño**, sin implementar (D2). Sin cifrado en 1.0 (D3) |
| **T-04 y H-2** | Un atacante que reescribe un bloque y recalcula el CRC: el motor no lo detecta en lectura | Manipulación silenciosa | [threat model §6.6](../security/threat-model.md). `resealed_zeroed_filter_is_refused_at_open_and_reported_by_verify` cubre el caso grueso | (a) No añadir integridad autenticada; `verify()` con `deep` como recurso. (b) Código de autenticación con clave en la ranura de 40 bytes, sin confidencialidad, con bit nuevo. (c) AEAD completo. Las opciones (b) y (c) cambian el formato y exigen custodiar una clave (D4) | Opción (a) de D4, tomada: sin integridad autenticada en 1.0 |
| **T-09** | Retención involuntaria: el borrado es lógico, y los bytes siguen en archivos vivos hasta compactar | El dato suprimido puede quedar en disco | [ADR-0009](../adr/0009-supresion-retencion-y-backups.md), [threat model §6.5](../security/threat-model.md) | Opción 3 de ADR-0009: sin borrado físico garantizado | Aceptada (ADR-0009, 2026-11-12) |
| **T-10** | La CI no ejecuta fuzzing ni auditoría de dependencias (`cargo audit`/`cargo deny`) | Cadena de suministro sin auditoría automática | [threat model §3](../security/threat-model.md) | Añadir a la CI. Ningún documento lo decide | No decidido |
| **Copias de seguridad** | No hay función de copia en el motor, y no se declara ventana de copias | Sin plazo de pérdida declarado | [ADR-0009](../adr/0009-supresion-retencion-y-backups.md), D11 y B-1 | Función de copia (D18), fuera de 1.0 | No decidido |

### 5.8 Documentación y coherencia

| Qué | Dónde | Estado |
|---|---|---|
| La tabla de la especificación dice que la Etapa 2 está «En curso», mientras el roadmap la marca como cerrada | [docs/spec/README.md](../spec/README.md) y [roadmap](../roadmap/README.md) | Sin corregir. El roadmap fija la fecha 2026-11-11 |
| El informe de la Fase 1 (2026-10-29) dice que ADR-0013 sigue «Propuesto» y que la congelación está pendiente | [f1-report §1 y §7](../testing/f1-report.md) | Superado por ADR-0013 (aceptado el 2026-10-31). El informe no se reescribe |
| El cierre de la Etapa 7 cita el nombre antiguo de una prueba | [cierre E7 §3.1](etapa7-cierre.md) | Renombrada en P1 |
| El `test-plan` tiene un marcador sin rellenar (`TIEMPO_SUITE`) donde debería ir la duración de la suite | [test-plan §7.5](../testing/test-plan.md) | Sin corregir |
| `T-CNT` no se reejecutó en el cierre de la Etapa 9 | [cierre E9 §3](etapa9-cierre.md) | Sin conteo global posterior a la Etapa 8 |
| ADR-0017 sigue «Propuesto», aunque su texto dice que se decide al iniciar la Etapa 3 | [ADR-0017](../adr/0017-semantica-de-la-unidad-neuronal.md) | Sin decidir (ver sección 8) |
| El índice de ADR marca ADR-0007 como «Pendiente»; su cuerpo dice «Aceptado» y no tiene fecha | [índice de ADR](../adr/README.md), [ADR-0007](../adr/0007-consistencia-de-indices.md) | Contradicción sin corregir |
| Algunos commits de `main` llevan trailers de autoría que las reglas del proyecto prohíben | [cierre E9 §5, punto 12](etapa9-cierre.md) | Mancha conocida y aceptada. No se reescribe la historia |

### 5.9 Inconsistencias de documentos que P4 encontró y nadie corrigió

P4 las encontró al auditar los requisitos de 1.0 ([requisitos-1.0](requisitos-1.0.md), filas 2 y 4). Este informe las comprobó contra los archivos citados el 2026-11-13. Ninguna está corregida: los textos afectados son de un acta cerrada y de un ADR aceptado.

| Id | Qué dice el documento | Qué dicen los archivos | Dónde está el texto | Por qué no se corrige sola |
|---|---|---|---|---|
| **(a)** Fila 15 de la acta, §2.4 | Que `set_policy` fija `KeepLast(1)` por tipo: «Resuelto por la API existente: la aplicación fija `KeepLast(1)` en su tipo». Cita `UnitDb::set_policy` en `db.rs:619` | `set_policy` está en `crates/kerno-units/src/db.rs:625` y recibe un `HistoryPolicy`. `HistoryPolicy` solo tiene `None`, `Audit` y `Versioned` (`crates/kerno-units/src/types.rs:144`). `KeepLast` es una variante de `RetentionRule` (`crates/kerno-units/src/retention.rs:67-72`), que recibe `run_retention`. El ejemplo poda con `prune_history` (`examples/agent-memory/src/lib.rs:576`) | [etapa9-acta.md](etapa9-acta.md), línea 119 | El acta está cerrada. El cierre de la Etapa 9 (§5, punto 8) ya anota la diferencia, y el acta no se corrige |
| **(b)** Criterio T5 de ADR-0020 | Que una build sin la característica rechaza al abrir una base con derivados, y que «el árbol de archivos (nombres y hash de cada uno) es idéntico antes y después» | Un binario de `d01f646` devuelve `IncompatibleLayer { found: 2, supported: 1 }`, pero el árbol no queda idéntico: `LOCK` se crea (0 bytes) y `MANIFEST` cambia su crc, con el mismo tamaño (254 bytes). `000003.sst` sí queda idéntico. La apertura del motor crea `LOCK` (`crates/kerno-core/src/vfs/std_fs.rs:136`, `create(true)`) y añade un registro de sesión a `MANIFEST`, antes de que la capa lea la marca ([etapa9-fixtures §3](etapa9-fixtures.md)) | [ADR-0020](../adr/0020-persistencia-de-vectores-como-derivados.md), línea 301 | ADR-0020 está aceptado. La regla del índice de ADR (sección 7) es no editar un ADR aceptado para cambiar una decisión, y el repositorio no dice cómo se corrige un criterio. Hay precedente de erratas registradas en otro documento (cierre E9 §7). El fixture registra la diferencia como salvedad, pero el criterio no se corrigió |

---

## 6. Riesgos y zonas sin información

Todo lo que sigue no se ha probado ni medido, o no hay información. Cada punto dice qué prueba o medición lo convertiría en conocimiento.

| Zona | Estado en el repositorio | Qué lo convertiría en conocimiento |
|---|---|---|
| **Plataformas** | Solo Linux con ext4, en una VM de 4 núcleos (kernel 6.18). Ningún documento prueba macOS, Windows ni otro sistema de archivos. La máquina objetivo (C-09) no está nombrada | Ejecutar la batería de la sección 9 en la máquina objetivo, y en un segundo sistema de archivos. Repetir T1 a T10 allí |
| **Dispositivo real con pérdida de páginas** | LazyFS pendiente, sin ejecuciones registradas | Ejecutar `lazyfs.yml` con FUSE, o el escenario A en un montaje propio |
| **Tamaños** | Calidad asociativa: 2 000 unidades. Rendimiento asociativo: 20 000 unidades. Derivados: 2 000 unidades (calidad) y 20 000 (rendimiento). Fase 1, escala `large` (1 000 000 de registros): solo `bulk_load` y `point_reads`; `range_scans` y `small_commits` no se ejecutaron | Ejecutar `derived_bench2`, `index_bench` y `run-benchmarks.sh` con `LARGE=1` a 100 000 y 1 000 000 unidades |
| **Concurrencia entre lectores** | Sin medir con varios lectores en la caché (cierre de la Etapa 6, §3.4 y §4.2: «sin concurrencia») | Un banco con varios lectores y escritores simultáneos |
| **Relaciones de bases anteriores a la capa 4** | Sin registro de conteo, una entrada de adyacencia perdida sigue siendo invisible desde su extremo, y no hay migración en bloque (I-18). Vale también para el doble daño | Escribir una migración que cuente las relaciones, o aceptar el límite. Ninguna está decidida ([units-v4 §7](../file-format/units-v4.md)) |
| **Multiescritor** | Fuera de alcance del motor (K-E3-2: la ranura de escritura es única). El group commit tiene pruebas con cuatro escritores | Un banco y una prueba de carga con más de un escritor, si se quiere soportar |
| **Ejecuciones largas** | Fuzzing de 30 minutos por objetivo. `kill -9` de 200 iteraciones en la CI semanal. Corridas largas de `wal`, `manifest` y `open` pendientes, dejadas para la máquina del propietario (sección 9.3). La prueba intermitente de espacio tenía una causa de temporización, corregida en P2 (sección 4.8) | Corridas largas de `wal`, `manifest` y `open` (sección 9.3). Un soak de días. Repetir la suite en varias semanas |
| **Uso real fuera del equipo** | Ningún documento registra uso fuera del equipo. El ejemplo `agent-memory` lo usa solo el equipo, con la API pública | Un usuario externo que construya algo con la API pública. D12 lo pide para 1.0 |
| **Cifrado y claves** | No hay cifrado, ni gestión de claves, ni la ranura está usada. K-1 a K-4 de ADR-0009 sin medir | Construir el diseño de ADR-0006 y medir K-1 a K-4 |
| **Integridad autenticada** | No hay. H-2 queda como límite aceptado | Construir un código de autenticación o AEAD, o aceptar el límite en 1.0 (D4, ya tomada) |
| **Borrado físico** | No hay garantía. C-5 (bloques liberados del medio) no se mide | Medir C-5 en el dispositivo (ADR-0009, K-1 a K-4) |
| **Calidad semántica de la recuperación** | Vectores deterministas con semilla. Conjunto sintético de un solo autor, con juicios de ese autor. Q4 no demostrada | El conjunto de E8-10: vectores de consulta en cada clase, juicios independientes y umbrales antes de medir |
| **Modelo de embeddings** | No se decidió un modelo (D5 (b) de la Etapa 8). No hay adaptador a un modelo real en el núcleo | Decidir el modelo y medir con vectores reales |
| **Memoria y cachés** | Sin medir (H-5 de la Etapa 9). Los resúmenes viven en memoria y no hay medición de su uso | Medir memoria por carga, con el tamaño de caché de la configuración |
| **Historial largo** | K-E3-4: las instantáneas largas impiden reclamar espacio. Sin medición con historial de años | Un banco de retención con instantáneas antiguas durante horas |
| **Compatibilidad de formatos entre versiones** | Formato v1 congelado; las pruebas de vectores fallan si un byte cambia. Formato de unidades v4 (P1): una base con una relación creada por esta build no abre con lectores de capa 1 a 3 ([units-v4 §2](../file-format/units-v4.md)). No hay una prueba de migración entre versiones, porque no hay migraciones (D11 deja un registro de migración sin escritor) | Escribir la migración y sus fixtures antes de 1.0 |
| **Exportación** | Formato borrador sin congelar (D10). I-30 verificada solo para el borrador | Congelar el formato solo después de D12 |
| **Backups** | Sin función de copia. Sin plazo declarado | Decidir una función de copia (D18) y medir su ventana |
| **Coste de la caché y del planificador en máquina en reposo** | Medido con carga. Pendiente la repetición | Repetir los banco de E5 y E6 sin carga |
| **Estabilidad de vectores entre plataformas** | Garantizada por diseño, no probada en otra plataforma (R-12 de la Etapa 7) | Ejecutar la prueba de vectores en otra plataforma |
| **Resultados frente a SQLite con umbrales** | No hay umbrales (D6). El informe es descriptivo | Fijar umbrales antes de medir, en calma |
| **Carga de la máquina en las mediciones** | Casi todas las mediciones se hicieron con otros procesos activos. P1 y P2 de la Etapa 8 no se midieron en reposo, y esa corrida está pendiente (sección 9.4) | Repetir sin carga, con `uptime` anotado en cada corrida |
| **Revisión independiente** | Pendiente para E9-16 y para P1, según [post9-pendientes](post9-pendientes.md) (punto 2 y su desglose). El registro de mutaciones de P1 cita una revisión independiente que halló un defecto, ya corregido (sección 4.7) | Una revisión hecha por alguien distinto del autor de P1 (acta §4.10) |
| **Verificación de este informe** | Ninguna prueba se ejecutó ni se compiló para escribirlo. Las cifras de P1 a P3 son las de sus documentos, con su fecha | Ejecutar la batería de la sección 9 en la máquina propia |
| **Documentos de investigación** | `docs/research/` y `experiments/` no se leyeron para esta nota | Una nota propia |

---

## 7. Decisiones tomadas y revocables

Las decisiones de esta sección son las que los documentos registran como tomadas por delegación del propietario, o aprobadas por él, con la marca de si son revocables. El texto de cada una está en el documento citado. Para cada una se dice qué haría falta para revocarla, según lo que el documento dice.

**Regla general para los ADR.** Un ADR aceptado no se edita para cambiar la decisión: se crea otro que lo reemplaza y se marca el anterior ([índice de ADR](../adr/README.md)). Para revocar un ADR, entonces, hace falta un ADR nuevo y, si la decisión tocó formato o código, el trabajo que lo aplica.

### 7.1 Decisiones del acta de la Etapa 9 (2026-11-13)

Tomadas el 2026-11-13 por delegación expresa del propietario, revocables por él, las D1, D2, D3, D4, D12, D15 y D16. Las demás, por el delegado del propietario, el 2026-11-12. [Acta §8.1](etapa9-acta.md).

| Decisión | Qué decide | Qué haría falta para revocarla |
|---|---|---|
| **D1** | Identificadores públicos: token aleatorio opaco con mapeo persistido (opción 1 de ADR-0005). E9-02 fusionado | Revocable mientras no se construya nada irreversible sobre ellos (según §8.1). Hoy la capa lleva la marca 3 y la exportación borrador lleva tokens (SAD 0.4.1, §13). Revocarla cambiaría ese formato; el documento no describe el procedimiento |
| **D2** | AEAD por bloque como **diseño** si se construye cifrado. Nada se implementa | Revocable mientras no se construya algo irreversible sobre él. Hoy no hay código de cifrado (§8.1); revocarla es un ADR nuevo, según la regla del índice |
| **D3** | Sin cifrado en 1.0. 1.0 no promete confidencialidad en reposo | Revocarla obliga a construir el cifrado antes de 1.0, con D2 y K-1 a K-4 medidos. Retrasa 1.0 |
| **D4** | Sin integridad autenticada en 1.0. `verify()` con `deep` es el recurso | Revocarla obliga a construir una autenticación de integridad, con su formato |
| **D5** | ADR-0012 aceptado tal como está (metodología de benchmarks) | Un ADR nuevo que cambie la metodología. Las mediciones ya hechas quedan con la metodología anterior |
| **D6** | Informe frente a SQLite sin umbrales | Fijar umbrales antes de medir, en calma |
| **D7** | Sin límite absoluto de latencia | Fijar un límite antes de medir. Un límite incumplido abriría una decisión nueva |
| **D8** | Sin formato en disco para resúmenes ni para auditoría en 1.0 | Un formato nuevo es un cambio de formato: ADR y marca de capa |
| **D9** | Crates actuales, sin fachada única | La fachada (ADR-0010 opción 1) es un cambio de API, anotable antes de 1.0 |
| **D10** | Exportación en borrador sin congelar | Congelar el formato, que es D12 |
| **D11** | Fixtures de v1; fixture de ADR-0020 reconstruido o declarado ausente | Escribir un registro de migración |
| **D12** | 1.0 se declara solo después de D13 y de un uso real fuera del equipo. **1.0 no se declara ahora** | Revocarla significaría declarar 1.0 antes de cumplir D13. Lo que cambia es la promesa de formato y de API |
| **D13** | La lista de requisitos de §8 como criterio de 1.0 | Cambiar la lista |
| **D14** | `Reclaim` público, con el texto de ADR-0009 | Cambiar el texto de la API (cualquier texto que diga «borrado seguro» o «borrado físico» sería una promesa falsa) |
| **D15** | Dirección: licencia amplia no exclusiva con derecho a relicenciar. **El texto no está en vigor**: hay un borrador sin vigor ([cla-propuesta](../legal/cla-propuesta.md), P4) | Cambiar la dirección exige otro texto legal. Ninguna contribución externa se acepta hasta que el propietario apruebe el texto |
| **D16** | ADR-0016 aceptado y confirmado con enmiendas tras revisar G1 a G7 | Revocarla es un ADR nuevo. Las tres carencias están cerradas en P3 (sección 4.9). Con el observador por debajo de `Fingerprint`, la caché no guarda nada |
| **D17** | Cláusula de C-4 de ADR-0009 sin reescribir; contradicción registrada | Reescribir la cláusula con un ADR nuevo |
| **D18** | Todas las clases de borrado en la opción 1; sin función de copia | Una función de copia y su ventana (B-1) |
| **D19** | I-28 no aplica en 1.0; I-42 e I-43 promovidas; I-29 para `kerno-units` | Revisar las promociones si las pruebas no se mantienen |
| **D20** | Aceptar H-3 con su prueba; exigir evidencia de F-10; revisar K-E3-7 | Reabrir cualquiera de los tres puntos |
| **D21** | Pruebas de propiedades con la biblioteca ya presente. Sin herramienta de fuzzing nueva en `kerno-units` | Añadir una herramienta de fuzzing nueva |
| **D22** | Umbrales de §5 del acta sin cambios antes de congelar | Cambiar umbrales (no debería hacerse después de medir) |
| **D23** | Escritura cruda y ganchos de prueba de `UnitDb` solo bajo `test-support` (apagada por defecto) | Exponer la escritura cruda fuera de `test-support`: cambio de la API pública |

### 7.2 Otras decisiones con delegación o aprobación

| Decisión | Quién y cuándo | Qué la hace revocable |
|---|---|---|
| Cierre de la Etapa 5 aprobado | Por delegación, 2026-11-09 | Un cambio en el cierre; las limitaciones H-1 y H-2 siguen registradas |
| Cierre de la Etapa 6 aprobado, con la caché **desactivada por defecto** | Por delegación, 2026-11-11 | Activar la caché por defecto exigiría una decisión sobre un criterio que la medición no permite afirmar |
| ADR-0011 (vectores exactos, recuperación híbrida), aceptado | Por delegación, 2026-11-11 | ADR nuevo |
| Etapa 7: ranking **desactivado por defecto** y experimental; T4 no se reescribe | El propietario, 2026-11-12 | Medir el ranking con un conjunto nuevo y un umbral fijado antes de medir |
| Etapa 8 cerrada como trabajo de ingeniería, con Q4 no demostrada | Por delegación, 2026-11-12 | Q4 se cierra con el paquete E8-10 |
| ADR-0012 aceptado (metodología de benchmarks) | Por delegación, 2026-11-12 | ADR nuevo |
| ADR-0018 (registro común de derivados), opción 2 | El propietario, 2026-11-12 | ADR nuevo |
| ADR-0009 (supresión), opción 3 | El propietario, 2026-11-12 | ADR nuevo. El ADR dice que sin plazo de backup declarado |
| ADR-0020 (persistencia de vectores como derivados), revocable por el propietario | Por delegación, 2026-11-12 | ADR nuevo que reemplace su cláusula. ADR-0011 ya lo marca como reemplazado en persistencia |
| K-E3-7 aceptado por diseño | El propietario, 2026-11-04 | Revisado en la Etapa 9, con K1 a K5 (ver sección 5) |
| Integrar la Etapa 3 con U2 y U6 incumplidos | El propietario, 2026-11-04 | Abaratar la materialización, o juzgar U2 sobre `history_page`. Cambia el criterio |
| Umbral frente a SQLite: solo informe | El propietario, 2026-11-11 | Fijar umbrales antes de medir |
| Comparación con SQLite, adelantada antes de la Etapa 8 | El propietario, 2026-11-11 | Igual que la anterior |
| Caso de uso real antes de 1.0 (memoria para un agente) | El propietario, 2026-11-11 | Hecho en E8-09 y E9-21; el uso fuera del equipo no existe (sección 6) |
| Aleatoriedad de los identificadores públicos: el token de 128 bits se lee de `/dev/urandom` con `std`, sin dependencia nueva. En una plataforma sin ese origen, `create_public` falla con un error tipado y no crea nada (`crates/kerno-units/src/ids.rs:283`) | Por delegación expresa del propietario, 2026-11-13 (P4) | Una API de aleatoriedad en la biblioteca estándar para las plataformas que el proyecto soporta, o aceptar una dependencia, lo que pasa por I-31 ([ADR-0005, «Notas»](../adr/0005-identificadores-publicos.md)) |
| Formato de unidades v4 (conteos de adyacencia), decisión de diseño | Tomada por delegación, 2026-11-13 (P1; [units-v4](../file-format/units-v4.md), «Estado») | Cambiarla es un cambio de formato, con su marca de capa 4 ([units-v4 §2 y §7](../file-format/units-v4.md)) |

---

## 8. Lo que sigue siendo del propietario

Estas decisiones no se han delegado, o los documentos las reservan al propietario en persona.

### 8.1 Decisiones reservadas

1. **Aprobar el texto del contrato de contribución (CLA) antes de aceptar cualquier contribución externa.** ADR-0019 lo exige, y D15 fija la dirección. **Hay un borrador, sin vigor** ([cla-propuesta](../legal/cla-propuesta.md), P4), que necesita la revisión de un abogado. No es asesoramiento jurídico. Mientras tanto, la única regla sobre contribuciones es la sección «Licencia» del README. Fuentes: [ADR-0019](../adr/0019-licencia-y-contribuciones.md), [acta §8.1, D15](etapa9-acta.md).

2. **Declarar 1.0.** Solo después de cumplir D13 y de tener un uso real fuera del equipo (D12). La lista de D13 tiene siete puntos. Estado de cada uno según los documentos:

   | Punto de D13 | Estado según los documentos |
   |---|---|
   | (1) Etapa 9 cerrada, con §4 y §5 cumplidos o declarados | **Cumplido con límites declarados** ([requisitos-1.0](requisitos-1.0.md), punto 1). Cierre aprobado el 2026-11-13. Quedan pendientes la revisión independiente de E9-16 y de P1, y el conteo de pruebas (T-CNT), que no se reejecutó. La medición de E9-20 y el fuzzing de E9-14 corrieron con carga alta |
   | (2) Los 15 puntos de fricción con disposición | **Cumplido con límites declarados** (requisitos-1.0, punto 2). La fila 15 no coincide con la acta (sección 5.9, a) |
   | (3) E8-10 cerrada con sus umbrales | **Cumplido con límites declarados** (requisitos-1.0, punto 3). E8-10 se midió una vez, con carga (E9-20). Los umbrales de calidad se imprimen y no se afirman. P1 y P2 no se midieron en reposo |
   | (4) Fixtures de v1 y el de ADR-0020 probados o declarados | **Cumplido con límites declarados** (requisitos-1.0, punto 4). El fixture de ADR-0020 está reconstruido. Su criterio T5 no se cumple como está escrito (sección 5.9, b) |
   | (5) I-30 verificada para el borrador y la reimportación | **Hecho para el borrador.** La congelación no se hace (D12) |
   | (6) Reclamación pública con el texto de ADR-0009 y su prueba | **Cumplido con límites declarados** (requisitos-1.0, punto 6). E9-07 e I-44. Sin borrado físico ni criptográfico, y sin plazo de copias |
   | (7) Límites declarados en el threat model | **Cumplido** (requisitos-1.0, punto 7). T-03, T-04, T-09 y T-10 siguen abiertas, y así está declarado |

   Además, D12 pide un uso real fuera del equipo, que no existe en los documentos. La declaración de 1.0 la hace el propietario.

3. **Revisar y aprobar la auditoría de requisitos para 1.0** ([requisitos-1.0](requisitos-1.0.md)), **el borrador del CLA**, que no tiene vigor, y **la decisión sobre la aleatoriedad de los identificadores** (sección 7.2). La auditoría no declara 1.0.

4. **Decidir sobre la revisión de contingencia de ADR-0001**, por el incumplimiento de T1 en la Fase 1. El informe lo deja como decisión abierta del propietario ([f1-report §8](../testing/f1-report.md)).

5. **Aprobar los objetivos T1 a T10.** Siguen «pendiente» en su documento ([f1-objetivos-rendimiento](../testing/f1-objetivos-rendimiento.md)). El informe de la Fase 1 lo lista como decisión abierta.

6. **Nombrar la máquina objetivo (C-09)** y repetir la medición allí. El informe de la Fase 1 lo lista como pendiente ([§8, punto 4](../testing/f1-report.md)).

7. **Decidir si los índices se mantienen dentro de cada `ChangeSet`** (ADR-0007). Hoy son derivados que solo `rebuild_index` escribe. El cierre de la Etapa 4 dice que la decisión requiere una carga mixta de lectura y escritura, que no existe ([cierre E4 §6](etapa4-cierre.md)).

8. **Decidir sobre H-5, H-6 y H-7 de la Etapa 6** y sobre la mejora del p99 de W1. El cierre de la Etapa 6 aceptó H-5, H-6 y H-7 como riesgos registrados ([cierre E6 §7](etapa6-cierre.md)). H-3 se cerró en P3 (sección 4.9), así que no queda decisión sobre ella.

9. **Decidir sobre K-E3-8 (U2).** Juzgar U2 sobre `history_page` sigue siendo decisión del propietario y cambia la definición de la decisión 12 de la Etapa 3. Abaratar la materialización de `Revision` es trabajo de ingeniería ([cierre E3 §3 y §5](etapa3-cierre.md)).

10. **Decidir sobre ADR-0006, ADR-0017 y ADR-0007 en su estado.** ADR-0006 está pendiente, aunque el diseño elegido ya está decidido (D2). ADR-0017 está «Propuesto», aunque su texto dice que se decide al iniciar la Etapa 3, que ya está cerrada. ADR-0007 dice «Aceptado» en su cuerpo, pero el índice de ADR lo marca «Pendiente».

11. **Repetir en reposo las mediciones de tiempo de la Etapa 8 (P1 y P2, cocientes)**, si se quiere cerrar que son fiables. El cierre de la Etapa 9 lo deja como pendiente. Se deja para la máquina del propietario (sección 9.4).
12. **Decidir cómo se corrigen las dos inconsistencias de la sección 5.9.** Una está en el acta de la Etapa 9, cerrada, y la otra en ADR-0020, aceptado. El repositorio no dice cómo se corrige un criterio de un ADR aceptado. Hay precedente de erratas registradas en otro documento (cierre E9 §7).
13. **Decidir quién hace la revisión independiente de P1 y la de E9-16.** [post9-pendientes](post9-pendientes.md) las da como pendientes, y el autor de P1 no puede ser su revisor (acta §4.10).
14. **Ejecutar en su máquina lo que se dejó para ella:** la corrida larga de `wal`, `manifest` y `open` (sección 9.3) y la re-medición en reposo de P1 y P2 de la Etapa 8 (sección 9.4). Ninguna de las dos se ha ejecutado.

### 8.2 Trabajo que los documentos mandan hacer antes de 1.0 (no es decisión del propietario, pero sí su responsabilidad de seguimiento)

- E8-10, medido una vez en E9-20 con vectores de consulta en cada clase. Sus umbrales de calidad se imprimen y no se afirman ([requisitos-1.0](requisitos-1.0.md), punto 3; [cierre E8 §8, (c)](etapa8-cierre.md)).
- Los quince puntos de fricción de `agent-memory` tienen disposición ([requisitos-1.0](requisitos-1.0.md), punto 2). El orden de trabajo propuesto está en el [cierre E8 §6](etapa8-cierre.md). La fila 15 no coincide con la acta (sección 5.9, a).
- Un uso real de la API fuera del equipo (D12).

---

## 9. Cómo continuar en una máquina propia

**Advertencia previa sobre el disco.** Un build de release completo necesita **varios GB de disco libre por copia** del repositorio. Ningún documento mide cuánto ocupa `target/`: esta cifra es una advertencia de planificación, no un dato medido. El informe de la Etapa 3 describe el disco de la VM de referencia con el 90 % ocupado ([etapa3-report](../testing/etapa3-report.md)), y la medición E9 de SQLite anota 7,1 GiB libres antes de medir ([sqlite-report-e9 §2](../testing/sqlite-report-e9.md)). Antes de compilar, comprobar el espacio libre con `df -h`. El disco de la máquina donde se escribió este informe estaba casi lleno y es compartido con otros procesos. Por eso la corrida larga de fuzzing y la re-medición en reposo se dejaron para la máquina del propietario (secciones 9.3 y 9.4).

**Requisitos.** Linux. Rust 1.89 o posterior, que es el `rust-version` del workspace. Las pruebas se hicieron con Rust 1.97. Un compilador de C, porque los crates de benchmark compilan SQLite. Fuente: [README](../../README.md), sección «Development». Para el fuzzing, un toolchain nightly y `cargo-fuzz` (ver más abajo).

**Clonar y entrar:**

```bash
git clone <url-del-repositorio> kerno
cd kerno
```

La URL no está en los documentos del repositorio. Hay que ponerla.

### 9.1 Comprobaciones de la CI (copiadas de `ci.yml` y del README)

Ninguna de estas órdenes se ha ejecutado en esta tarea. La tarea prohíbe compilar. Están copiadas tal cual de [`ci.yml`](../../.github/workflows/ci.yml) y del README.

```bash
# formato y lints
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings

# guardas de dependencias y de acceso a archivos (cada una debe terminar con código 0)
test "$(cargo tree -p kerno-core -e normal --prefix none | wc -l)" -eq 1
tree=$(cargo tree -p kerno-storage -e normal --prefix none) && grep -q '^kerno-storage ' <<<"$tree" && ! grep -E '^(spike-|kerno-testkit|proptest|loom)' <<<"$tree"
if grep -rn 'std::fs' crates/kerno-storage/src; then false; else test $? -eq 1; fi

# pruebas: todo el workspace salvo kerno-storage
cargo test --workspace --exclude kerno-storage --all-features --release -- --nocapture

# pruebas: kerno-storage
cargo test -p kerno-storage --all-features --release -- --nocapture
```

**Duraciones.** Los documentos dan estas cifras:

- `cargo clippy` terminó en 20,43 s en la E6-05 y en 30,52 s en la E7-06, en la VM de referencia. Son tiempos de una compilación incremental, no de una limpia ([cierre E6 §5](etapa6-cierre.md), [cierre E7 §5](etapa7-cierre.md)).
- El presupuesto de la CI es de 15 minutos para `test-spikes` y 25 para `test-kerno` ([test-plan §7.5](../testing/test-plan.md)). El propio documento no tiene la duración medida: el marcador `TIEMPO_SUITE` no está rellenado.
- **No hay duración documentada** para la suite completa, ni para la compilación limpia.

**Pruebas largas y semanales (copiadas de la CI):**

```bash
# semilla larga: 1 000 semillas en las suites aleatorias (CI semanal)
SPIKE_SEEDS=1000 cargo test --workspace --all-features --release -- --nocapture

# kill -9: 200 iteraciones (CI semanal)
KERNO_KILL_ITERS=200 cargo test --release -p kerno-storage --test process_kill -- --nocapture

# modelos de loom del canal de commits (CI semanal, sin duración documentada)
RUSTFLAGS="--cfg kerno_loom" cargo test -p kerno-storage --release --lib commit_queue
```

**Humo de benchmarks (copiado de la CI):**

```bash
cargo build --release -p spike-bench --features candidates,kerno
target/release/spike-bench suite --engines btree,lsm,kerno,redb,fjall,sqlite --workloads all --scales tiny --reps 1 --dir bench-data --out smoke.jsonl
target/release/spike-bench report --input smoke.jsonl
! grep -q '"error"' smoke.jsonl
cargo run --release -p kerno-units --features test-support --example unit_bench -- --quick
```

Ninguno de estos comandos se ha ejecutado en esta tarea.

### 9.2 Benchmarks completos de M0

```bash
spikes/storage/run-benchmarks.sh
SCALES=small,medium,large REPS=5 spikes/storage/run-benchmarks.sh
```

Variables documentadas en el propio script ([`run-benchmarks.sh`](../../spikes/storage/run-benchmarks.sh)): `SCALES` (por defecto `small,medium`), `REPS` (por defecto 3), `ENGINES`, `COLD` (por defecto 1, **necesita root** para vaciar la caché del sistema), `LARGE` y `LARGE_WORKLOADS`. La duración no está documentada. M0 hizo 410 ejecuciones.

### 9.3 Fuzzing

Instalación, según [`fuzz/README.md`](../../crates/kerno-storage/fuzz/README.md):

```bash
rustup toolchain install nightly --profile minimal
cargo install cargo-fuzz --locked
```

Ejecución desde `crates/kerno-storage`. El corpus comprometido (`fuzz/corpus/<objetivo>`) **solo se lee**. Las entradas nuevas van a un directorio de trabajo aparte, que el owner elige:

```bash
mkdir -p <directorio-de-trabajo>/varint
cargo +nightly fuzz run varint <directorio-de-trabajo>/varint fuzz/corpus/varint -- -max_total_time=60 -timeout=1
# open necesita una entrada mayor:
cargo +nightly fuzz run open <directorio-de-trabajo>/open fuzz/corpus/open -- -max_total_time=60 -max_len=65536 -timeout=1
```

Duración por objetivo: 60 segundos en la prueba corta. Los documentos usan `-max_total_time=1800` para la corrida de treinta minutos, y las corridas registradas duraron 1 801 segundos cada una.

**Dejada para la máquina del propietario, a propósito.** Los documentos no dan una duración para esta corrida. Lo que dicen es que treinta minutos es poco y que una corrida más larga de esos tres es el paso natural ([cierre E9 §5, punto 7](etapa9-cierre.md)). No se hizo aquí porque el disco de la máquina de compilación estaba casi lleno y es compartido. El comando es el de la corrida de treinta minutos, desde `crates/kerno-storage`, como la orden de 60 segundos de [`fuzz/README.md`](../../crates/kerno-storage/fuzz/README.md) («Run»). Ese README escribe `corpus/<target>` en su orden de 1 800 segundos, que solo resuelve desde `fuzz/`; aquí se usa `fuzz/corpus/<target>`. El tiempo lo elige el propietario. Aquí figura `1800`, el valor de la última corrida registrada:

```bash
cd crates/kerno-storage
mkdir -p <directorio-de-trabajo>/wal
cargo +nightly fuzz run wal <directorio-de-trabajo>/wal fuzz/corpus/wal -- -max_total_time=1800 -max_len=65536 -timeout=1
mkdir -p <directorio-de-trabajo>/manifest
cargo +nightly fuzz run manifest <directorio-de-trabajo>/manifest fuzz/corpus/manifest -- -max_total_time=1800 -max_len=65536 -timeout=1
mkdir -p <directorio-de-trabajo>/open
cargo +nightly fuzz run open <directorio-de-trabajo>/open fuzz/corpus/open -- -max_total_time=1800 -max_len=65536 -timeout=1
```

Para una corrida más larga, sustituir `1800` por el número de segundos que se elija. Un fallo deja un archivo en `fuzz/artifacts/<objetivo>/`. Se reproduce con `cargo +nightly fuzz run <objetivo> fuzz/artifacts/<objetivo>/<archivo>`, según el README.

**Reproducir un fallo ya guardado.** Ver el README, sección «Reproduce an artifact».

### 9.4 Re-medición en reposo, pendiente, dejada para la máquina del propietario

Los cocientes P1 y P2 de la Etapa 8 se volvieron a medir en E9-20, sobre el conjunto de E8-10, con otro proceso activo en la máquina. El cierre de la Etapa 9 pide repetirlos con la máquina en reposo. Esa corrida no se hizo aquí, por la misma razón que la de la sección 9.3. Los comandos son los de la salida cruda ([etapa9-medicion-salida.txt](etapa9-medicion-salida.txt)), líneas 4 y 180:

```bash
uptime
# calidad del conjunto E8-10 (prueba; comprueba el MANIFEST y las cifras)
cargo test -p kerno-units --release --test derived_measure2 -- --nocapture --exact quality_over_the_second_frozen_set

# tiempos P1 y P2 (ejemplo, no se ejecuta con cargo test)
cargo run --release -p kerno-units --example derived_bench2 -- --runs 4 --reps 12 --dir <directorio-temporal>
```

Antes de cada corrida, anotar `uptime` (carga de 1 minuto). El propio informe da el criterio: T-SQ1 pide carga de 1 minuto por debajo de 4. El cierre de la Etapa 9 pide la máquina libre, y ese es el criterio más estricto.

Otras mediciones pendientes:

- **Escala `large` de `range_scans` y `small_commits`** (Fase 1). El informe dice: «repetir con `LARGE=1` en una máquina en reposo» ([f1-report §8, punto 6](../testing/f1-report.md)). El script `run-benchmarks.sh` acepta `LARGE_WORKLOADS`, así que una forma es `SCALES=medium LARGE=1 LARGE_WORKLOADS=range_scans,small_commits spikes/storage/run-benchmarks.sh`. **Esta combinación no está en ningún documento, y no se ha ejecutado.**
- **LazyFS**, en una máquina con FUSE: `./spikes/storage/run-lazyfs.sh`. El script devuelve 2 si faltan FUSE o las herramientas. El workflow `lazyfs.yml` se puede lanzar a mano.
- **Máquina objetivo (C-09)**: repetir T1 a T10 allí.

### 9.5 Prueba intermitente

`space_is_reclaimed_after_range_deletes` tenía una suposición de temporización en la propia prueba, corregida en P2 (sección 4.8). Si vuelve a fallar, repetir la suite de `kerno-storage` y comprobar que `settled_size` sigue esperando a que el tamaño baje de la cota. La intermitencia de la Etapa 4 (M-11) no tiene corrección (sección 5.6).

### 9.6 Reglas al medir

- Medir con la máquina en reposo y anotar `uptime` antes de cada corrida.
- No compilar en paralelo a la medición. Los documentos muestran que la carga cambia las cifras (E9-20, [medición §1](etapa9-medicion.md)).
- Guardar la salida cruda en el repositorio, como hacen los documentos.

---

## 10. Mapa de documentos

Una línea por documento.

| Documento | Pregunta que responde |
|---|---|
| [README](../../README.md) | ¿Qué es Kerno y en qué estado está? |
| [SAD](SAD.md) | ¿Cuál es la arquitectura de referencia y qué decisiones cambian ese documento? (versión 0.4.1) |
| [Roadmap](../roadmap/README.md) | ¿Qué etapa está cerrada, qué queda y qué decisiones siguen abiertas? |
| [Especificación v0.2.0](../spec/especificacion-arquitectonica-v0.2.0.md) | ¿Qué pide la especificación, sin editar? |
| [Correspondencia especificación–SAD](../spec/README.md) | ¿Qué etapa de la especificación es qué fase del SAD? |
| [Índice de ADR](../adr/README.md) | ¿Qué decisiones están aceptadas, pendientes o reemplazadas? |
| [ADR-0001](../adr/0001-motor-de-almacenamiento.md) | ¿Por qué el motor es un LSM nivelado? |
| [ADR-0002](../adr/0002-transacciones-y-durabilidad.md) | ¿Qué significa «commit» y qué garantiza cada modo de durabilidad? |
| [ADR-0003](../adr/0003-formato-fisico-y-versionado.md) | ¿Qué reglas de versionado e integridad tiene el formato? |
| [ADR-0005](../adr/0005-identificadores-publicos.md) | ¿Cómo son los identificadores públicos? (token opaco, D1; límite de aleatoriedad del 2026-11-13) |
| [ADR-0006](../adr/0006-cifrado-y-gestion-de-claves.md) | ¿Qué diseño de cifrado se eligió y por qué no se implementa? (pendiente) |
| [ADR-0009](../adr/0009-supresion-retencion-y-backups.md) | ¿Qué significa «borrar» en Kerno? (sin borrado físico garantizado) |
| [ADR-0011](../adr/0011-vectores-y-recuperacion-hibrida.md) | ¿Qué búsqueda vectorial hay? (exacta, sin índice aproximado) |
| [ADR-0012](../adr/0012-metodologia-de-benchmarks.md) | ¿Cómo se mide y con qué umbrales? |
| [ADR-0013](../adr/0013-formato-v1-y-protocolo-del-motor.md) | ¿Qué formato v1 está congelado? |
| [ADR-0014](../adr/0014-representacion-fisica-de-relaciones.md) | ¿Cómo se guardan las relaciones? |
| [ADR-0015](../adr/0015-privacidad-de-la-observabilidad.md) | ¿Qué puede contener una observación? (nada de contenido fuera de `diagnostic`) |
| [ADR-0016](../adr/0016-adaptacion-e-invalidacion.md) | ¿Qué puede aprender el motor y cómo se invalida? Confirmado el 2026-11-13; el trabajo abierto 1 a 3 está cerrado en P3 |
| [ADR-0018](../adr/0018-versionado-y-reconstruccion-de-derivados.md) | ¿Cómo se registran los derivados y cuándo se reconstruyen? |
| [ADR-0019](../adr/0019-licencia-y-contribuciones.md) | ¿Bajo qué licencia y con qué contrato se acepta código externo? (hay un borrador de CLA sin vigor) |
| [ADR-0020](../adr/0020-persistencia-de-vectores-como-derivados.md) | ¿Cómo se persisten los vectores como derivados? |
| [Invariantes](invariants.md) | ¿Qué no puede romper ningún cambio, y qué prueba lo verifica? (I-01 a I-45) |
| [Matriz de fallos](failure-matrix.md) | ¿Qué queda tras reabrir en cada punto de una caída? |
| [Cierre de la Etapa 3](etapa3-cierre.md) | ¿Qué construyó la capa de unidades y qué riesgos heredó? |
| [Cierre de la Etapa 4](etapa4-cierre.md) | ¿Cómo funcionan las relaciones y los índices, y qué queda abierto? |
| [Cierre de la Etapa 5](etapa5-cierre.md) | ¿Cómo explica el planificador sus planes y qué cuesta observar? |
| [Cierre de la Etapa 6](etapa6-cierre.md) | ¿Por qué la caché está apagada y qué no se afirma? |
| [Cierre de la Etapa 7](etapa7-cierre.md) | ¿Qué calidad y rendimiento tiene la memoria asociativa, y por qué el ranking está apagado? |
| [Cierre de la Etapa 8](etapa8-cierre.md) | ¿Qué derivados hay, qué no está demostrado (Q4) y qué sigue antes de 1.0? |
| [Cierre de la Etapa 9](etapa9-cierre.md) | ¿Qué queda cerrado, qué decisiones se tomaron por delegación y qué límites siguen? |
| [Acta de la Etapa 9](etapa9-acta.md) | ¿Qué paquetes y decisiones tenía la última etapa, y qué exige 1.0 (D13)? |
| [Revisión de K-E3-7](etapa9-k-e3-7.md) | ¿Qué lecturas comprueban menos que `verify_units`, y cuáles son K1 a K5? |
| [Pendientes posteriores a la Etapa 9](post9-pendientes.md) | ¿Qué punto del cierre sigue abierto y cuál cambió? |
| [Mutaciones posteriores](post9-mutaciones.md) | ¿Qué mutaciones hicieron P1 (M1 a M20), P2 (Q1 a Q16) y P3 (M1 a M17), y cuáles detectaron? |
| [Requisitos de 1.0](requisitos-1.0.md) | ¿Qué requisitos de D13 cumple el repositorio, y con qué límites? (auditoría de P4) |
| [Borrador del CLA](../legal/cla-propuesta.md) | ¿Qué contrato de contribución se propone? (borrador sin vigor, no aprobado) |
| [Formato de unidades v4](../file-format/units-v4.md) | ¿Qué añade el formato v4 (conteos de adyacencia) y qué límites tiene? (P1) |
| [Formato v1 del motor](../file-format/v1.md) | ¿Cómo se leen los bytes del motor? (congelado) |
| [Formato de unidades v1](../file-format/units-v1.md) | ¿Cómo se codifican las unidades, revisiones y relaciones? |
| [Formatos de unidades v2 y v3](../file-format/units-v2.md) | ¿Qué cambió en las versiones 2 y 3 de la capa? (v2: vectores persistidos; v3: identificadores públicos) |
| [Exportación, borrador](../file-format/export-v0-borrador.md) | ¿Cómo se exporta una base? (sin congelar) |
| [Plan de pruebas](../testing/test-plan.md) | ¿Qué pruebas hay, dónde corren y qué puertas exige la CI? |
| [Informe de M0](../testing/m0-report.md) | ¿Qué se midió al elegir el motor? |
| [Informe de la Fase 1](../testing/f1-report.md) | ¿Qué salió de las pruebas y las mediciones de la Fase 1, y qué quedó pendiente? |
| [Revisión adversarial de la Fase 1](../testing/f1-adversarial-review.md) | ¿Qué defectos bloqueaban la fusión y cómo se corrigieron? |
| [Informe SQLite E9](../testing/sqlite-report-e9.md) | ¿Cómo se compara Kerno con SQLite, sin umbrales? |
| [Medición E8-10 (E9-20)](etapa9-medicion.md) | ¿Qué miden los cocientes P1 y P2 de la Etapa 8, y en qué condiciones? |
| [Threat model](../security/threat-model.md) | ¿Qué amenazas se consideran, cuáles siguen abiertas y qué queda fuera? |
| [Fuzzing](../../crates/kerno-storage/fuzz/README.md) | ¿Cómo se ejecuta el fuzzing y qué resultados tiene? |
| [Spike de almacenamiento](../../spikes/storage/README.md) | ¿Cómo se reproducen los benchmarks de M0? |
| [Ejemplo `agent-memory`](../../examples/agent-memory/FRICTION.md) | ¿Qué fricción encontró el uso real del equipo con la API pública? (quince puntos) |
| [CI](../../.github/workflows/ci.yml) | ¿Qué corre en cada cambio y qué corre cada semana? |
| [Contribuir](../../CONTRIBUTING.md) | ¿Qué reglas de código y de documentación hay para contribuir? |

---

## Cómo se comprobó esta nota

**Lo que se hizo.** Se leyeron los documentos de P1 a P4 y los citados en la cabecera. Las afirmaciones sobre el código se comprobaron con búsquedas de texto, sin compilar, con los comandos siguientes. Las salidas son las de esta actualización:

```text
$ git rev-parse --short HEAD
758b5d4

$ git cat-file -t 0c2c3d5; git cat-file -t 2356ff3; git cat-file -t fc7e233; git cat-file -t f2aa157
commit
commit
commit
commit

$ grep -n 'HistoryPolicy {' crates/kerno-units/src/types.rs
144:    HistoryPolicy { None = 0, Audit = 1, Versioned = 2 }

$ grep -rn 'KeepLast' crates/kerno-units/src/types.rs; echo "exit=$?"
exit=1

$ grep -n 'adjust_count' crates/kerno-units/src/db.rs
1486:        self.adjust_count(from, Direction::Out, relation_type, 1)?;
1487:        self.adjust_count(to, Direction::In, relation_type, 1)?;
1508:        self.adjust_count(r.from, Direction::Out, &r.relation_type, -1)?;
1509:        self.adjust_count(r.to, Direction::In, &r.relation_type, -1)?;

$ grep -n 'create(true)' crates/kerno-core/src/vfs/std_fs.rs
136:        let f = match fs::OpenOptions::new().read(true).write(true).create(true).truncate(false).open(path) {

$ grep -rn 'no source of derived datum' crates/kerno-units/src
crates/kerno-units/src/derived_stale.rs:124:                write!(f, "no source of derived datum {} is valid any more", IdHex(id))

$ grep -n 'T5 |' docs/adr/0020-persistencia-de-vectores-como-derivados.md | cut -c1-120
301:| T5 | **Una build sin la característica rechaza al abrir una base con derivados.** Un binario construido desde `d0

$ for n in a_refused_create_leaves_no_count the_created_record_carries_the_evidence_and_the_measured_cost replacing_the_key_leaves_a_record_of_the_wipe space_is_reclaimed_after_range_deletes; do grep -rl "$n" crates examples; done
crates/kerno-units/tests/rel_counts.rs
crates/kerno-units/src/cache.rs
crates/kerno-units/tests/cache_planner.rs
crates/kerno-testkit/src/harness/range.rs
crates/kerno-testkit/src/harness/mod.rs

```

Los nombres de prueba citados en esta actualización existen en el árbol (salida anterior y las demás comprobadas con el mismo método). Se buscaron en el informe nombres de modelos, de asistentes y de sus fabricantes, y no aparece ninguno. `completeness_is_not_a_query_time_check` ya no existe: se renombró en P1. Las líneas de código citadas en esta actualización se comprobaron con búsquedas de texto; las de las secciones 3 a 5 que no aparecen en el bloque se toman del documento de origen.

**Lo que no se hizo.** No se compiló ni se ejecutó ninguna prueba: la tarea lo prohíbe, y el disco de la máquina de compilación estaba casi lleno y es compartido. Por eso los comandos de la sección 9 no se han ejecutado: están copiados de la CI, de los READMEs y de la salida cruda de E9-20. Las cifras de las secciones 3 y 5 son las que dicen los documentos, con su fecha, y no se han repetido. Los paquetes P1 a P4 se describen según sus documentos y según el código que citan. El único archivo modificado es este informe. `experiments/narrative-threads/.venv` ya estaba sin seguimiento y no se tocó.
