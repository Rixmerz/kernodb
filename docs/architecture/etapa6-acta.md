# Acta de entrada de la Etapa 6: primera plasticidad operacional

| Campo | Valor |
|---|---|
| Fecha | 2026-11-09 |
| Estado | **Aprobada** el 2026-11-09 (decisión delegada por el propietario) |
| Acto | Propuesta de alcance y de los paquetes E6-00 a E6-05 |
| Documentos | [Roadmap, Etapa 6](../roadmap/README.md#etapa-6--primera-plasticidad-operacional) · [cierre de la Etapa 5](etapa5-cierre.md) · [ADR-0016](../adr/0016-adaptacion-e-invalidacion.md) (aceptado, G1–G7) · [ADR-0015](../adr/0015-privacidad-de-la-observabilidad.md) · [ADR-0018](../adr/0018-versionado-y-reconstruccion-de-derivados.md) · [ejecución adaptativa](adaptive-execution.md) · [invariantes](invariants.md) · spec v0.2.0 §9, §15.5, §16 |

## 0. Estado

Esta acta está **aprobada** (§8). Los WP son secuenciales, como en las Etapas 3 a 5.

## 1. Condición de inicio

| Condición | Estado |
|---|---|
| Etapa 5 cerrada | Cumplida: [cierre](etapa5-cierre.md) aprobado el 2026-11-09, con la promoción de I-32 (planificador) e I-37 y las limitaciones H-1 y H-2 |
| ADR-0016 (adaptación e invalidación) | *Aceptado* el 2026-11-09, sujeto a G1–G7 |
| ADR-0018 (versionado de derivados) | *Propuesto*. Solo hace falta si el estado aprendido se persiste (§7, punto 3) |
| Experimento de hilos narrativos | Cerrado con NO-GO ([informe](../../experiments/narrative-threads/results/REPORT.md)); no afecta a esta etapa ni al formato persistente |

## 2. Alcance

### 2.1 Objetivos

1. **Una sola** optimización reversible (roadmap): un plan reutilizable **o** una caché de resultados invalidada por secuencia de commit. Cuál de las dos es una decisión del propietario (§7, punto 1).
2. Los metadatos de spec §9 para esa optimización: dependencias declaradas con versión, evidencia de creación, versión de la optimización y del planificador.
3. Desactivación global y por consulta (I-36), invalidación por dependencia antes de cada uso (I-35, G4) y fallback registrado (I-33).
4. Mejora neta medida frente al modo desactivado, con umbrales fijados antes de medir.

### 2.2 Punto de partida

Tras la Etapa 5, `kerno-units` tiene un planificador determinista (`planner.rs`) sobre cuatro caminos (referencia, `IDX_PRIMARY`, `IDX_TYPE`, adyacencia), un `Plan` explicable, una huella con clave local (`fingerprint.rs`) y `ExecutionObservation` en memoria con niveles `off` a `diagnostic`. `check_units` y `check_relations` validan corrección parcial, no completitud (H-1). Las causas de fallback `precondition_failed`, `invalidated`, `disabled` y `discrepancy` y el `plan_kind` `learned`/`cache` existen solo en el diseño ([ejecución adaptativa](adaptive-execution.md)).

### 2.3 Fuera de alcance

- Una segunda optimización, o cualquier optimización que cambie el resultado o lo aproxime.
- La excepción «salvo prueba de que los datos leídos no cambiaron» de la caché (eliminada por G4 hasta un ADR posterior).
- Índices o caminos nuevos, estadísticas sin consumidor (spec §16).
- Persistir el estado aprendido, salvo decisión del propietario (§7, punto 3).
- Recuperación narrativa o asociativa: Etapa 7; el experimento de hilos no se integra.

## 3. Paquetes de trabajo

Todos son secuenciales. «Hecho» incluye `cargo fmt --check`, `cargo clippy --workspace --all-targets --all-features -- -D warnings`, la suite del WP y la suite completa en verde, el conteo de pruebas antes y después, y cada prueba nueva vista fallar por mutación.

| WP | Contenido | Depende de | Aceptación |
|---|---|---|---|
| **E6-00** Puertas de entrada | Aprobación del cierre de la Etapa 5 y de esta acta. Decisiones de §7 registradas. Re-medición del planificador y de `diagnostic` sobre máquina sin carga (riesgo heredado del cierre §5), con salida cruda comprometida. Medición base del modo desactivado para la carga que usará E6-04 | Etapa 5 cerrada | Acta aprobada; umbrales de §6 fijados; salidas comprometidas; ningún WP posterior arranca antes |
| **E6-01** Modelo de dependencias y estado aprendido | Tipo de optimización con sus dependencias versionadas (G4): versión de esquema, definición de índice y su watermark, estadísticas, precondiciones y, para una caché, la `commit_seq` de su instantánea. Almacén del estado aprendido separado de los datos de usuario, fuera de toda transacción (ADR-0016, decisión). Interruptores global y por consulta. Registro de transiciones (G7) con las reglas de ADR-0015 | E6-00 | Cada dependencia declarada tiene una prueba que la cambia e invalida la optimización (I-35); vectores fijos de dependencias; ninguna dependencia sin versión |
| **E6-02** Uso en el planificador | La optimización como candidato del planificador: se usa solo si las precondiciones actuales se comprueban en ese uso (G3); su respuesta pasa por `check_units`/`check_relations` (G5); cualquier fallo, discrepancia o duda ejecuta la ruta de referencia con causa registrada (`precondition_failed`, `invalidated`, `disabled`, `discrepancy`) (G2, I-33). La explicación dice si usó estado aprendido y cuál (G7) | E6-01 | Prueba diferencial con la optimización activada, desactivada e invalidada a mitad de ejecución (G1, I-32); inyección de precondiciones falsas y de estado corrupto (G2, G3); mutaciones que quitan una comprobación en la ruta aprendida mueren (G5) |
| **E6-03** Reversibilidad y desactivación | Activar, desactivar, invalidar y borrar el estado no toca datos confirmados, revisiones, relaciones, índices ni `commit_seq` | E6-02 | I-34 (parte de optimizaciones) e I-36: suite completa con la adaptación desactivada; estado lógico, bytes del motor y `commit_seq` iguales antes y después; con la adaptación desactivada ninguna optimización participa (contador a cero) |
| **E6-04** Medición y centinelas | Mejora neta frente al modo desactivado con la carga y los umbrales de §6. Centinelas (I-37) en registros, explicaciones y errores nuevos, en cada nivel. Prueba que recorre cada transición de G7 y encuentra su registro | E6-03 | Mediciones con salida cruda comprometida; centinelas en verde y muertos por la mutación «copiar un literal en el registro de la optimización» |
| **E6-05** Cierre | Revisión adversarial. Promoción de I-33…I-36 (y de I-34 completa). Actualización del roadmap, SAD §9 y `adaptive-execution.md` | todos | Criterios de §4 cumplidos; informe de revisión sin bloqueantes abiertos |

Camino crítico: E6-00 → E6-01 → E6-02 → E6-03 → E6-04 → E6-05.

## 4. Criterios de aceptación

Del [roadmap, Etapa 6](../roadmap/README.md#etapa-6--primera-plasticidad-operacional) y de ADR-0016:

| Criterio | Invariante / garantía | Evidencia prevista | Estado |
|---|---|---|---|
| Mejora neta medida frente al modo desactivado, con umbrales fijados antes | — | E6-04, umbrales de §6 | pendiente |
| Equivalencia de resultados | I-32, G1 | Prueba diferencial activada / desactivada / invalidada a mitad (E6-02); no se apoya en comprobaciones de completitud en consulta (H-1) | pendiente |
| Fallback a un plan válido, registrado, ante precondición, invalidación, desactivación o discrepancia | I-33, G2, G3 | Inyección de fallos y observaciones falsificadas (E6-02) | pendiente |
| Invalidación probada por cada dependencia | I-35, G4 | Un caso por dependencia declarada (E6-01) | pendiente |
| Invalidar, desactivar o borrar no altera datos | I-34, G6 | E6-03 | pendiente |
| Desactivación verificada | I-36, G6 | E6-03 | pendiente |
| La ruta aprendida pasa las mismas validaciones | G5 | Mutaciones (E6-02) | pendiente |
| Trazabilidad sin literales | G7, I-37 | Recorrido de transiciones y centinelas (E6-04) | pendiente |

## 5. Riesgos

| ID | Riesgo | Tratamiento propuesto |
|---|---|---|
| K-E3-8, K-E3-9 | U2 en `history`; U6 con retención concurrente | Sin cambios; esta etapa no escribe en el motor. Ningún umbral se rebaja |
| Heredado (cierre E5 §5) | Coste del planificador y de `diagnostic` con incumplimientos puntuales | Re-medir en E6-00 sobre máquina sin carga |
| H-1 (heredado) | Las comprobaciones en consulta no garantizan completitud | La equivalencia de la optimización se demuestra solo con la prueba diferencial (G1) |
| **Nuevo** | La ganancia sea pequeña: el planificador cuesta poco (+0,3 % a +14 % de p99), así que un plan reutilizable ahorra a lo sumo ese margen; una caché ligada a `commit_seq` solo acierta sin escrituras intermedias | Elegir la optimización con datos de E6-00 (§7, punto 1); si la mejora no alcanza el umbral, la etapa cierra con el resultado negativo documentado y la optimización desactivada por defecto |
| **Nuevo** | El estado aprendido crece sin cota | Cota por tamaño y edad, como las observaciones (E5-03) |
| **Nuevo** | Una caché de resultados guarda contenido de usuario en memoria | Fuera del alcance de la observabilidad (no es un registro), pero sí de ADR-0009 (supresión) si se persiste: por eso la propuesta es solo en memoria |

Política de merge: la [puerta de corrección de la Fase 1](phase1-design.md#71-puerta-de-corrección-adr-0012-umbral-1-c-25-bloqueante) es bloqueante. Los incumplimientos de rendimiento se registran como riesgos y no bloquean, salvo el criterio de mejora neta, que decide si la optimización queda activada por defecto.

## 6. Umbrales de rendimiento

T1–T10 y U1–U6 no cambian. Propuesta, a fijar en E6-00 antes de medir:

- Mejora neta: p50 de la carga de lecturas repetidas de W1 con la optimización ≥ 20 % mejor que desactivada; p99 no peor que desactivada en más de 5 %.
- Con una carga sin repetición (peor caso), sobrecoste ≤ 5 % de p99 frente a desactivada.
- Escrituras: sin cambio medible en U1 (la optimización no escribe en el motor).

## 7. Puntos que el propietario debe decidir

1. **Qué optimización.** (a) Plan reutilizable por huella: invalida con versiones de esquema, índice y estadísticas; ahorra solo el coste de planificar. (b) Caché de resultados por huella y `commit_seq`: invalida exactamente por secuencia (G4); acierta solo en lecturas repetidas sin escrituras entre medias. **Recomendación: (b)**, porque su invalidación es la más simple de demostrar (una sola dependencia exacta) y su ganancia potencial es mayor que el coste del planificador; la decisión final se apoya en la medición base de E6-00.
2. **Estado por defecto.** Propuesta: desactivada por defecto hasta que E6-04 muestre la mejora neta; activada solo si cumple §6.
3. **Dónde vive el estado aprendido.** Propuesta: en memoria, con cota de tamaño y edad. Persistirlo exige ADR-0018 aceptado y ADR-0009 (supresión).
4. **Umbrales de §6**, fijados antes de medir.
5. **Carga de medición.** Propuesta: W1 con proporción de consultas repetidas fija (por ejemplo 80 % repetidas y 20 % escrituras intercaladas), más la carga sin repetición como peor caso.

## 8. Aprobación

| Fecha | Decisión | Notas |
|---|---|---|
| 2026-11-09 | **Aprobada** | El propietario delegó las decisiones («lo mejor y más correcto, toma tú las riendas»). Decisiones de §7: (1) caché de resultados por huella y `commit_seq` (opción b), confirmable con la medición base de E6-00: confirmada por la [re-medición de E6-00](etapa6-e600-remedicion.md): el planificador cuesta ≈ 0 % en p50, así que un plan reutilizable no puede alcanzar §6; (2) desactivada por defecto hasta cumplir §6; (3) estado solo en memoria con cota de tamaño y edad; (4) umbrales de §6 tal como están, fijados antes de medir; (5) carga: W1 con 80 % de lecturas repetidas y 20 % de escrituras intercaladas, más la carga sin repetición como peor caso |
