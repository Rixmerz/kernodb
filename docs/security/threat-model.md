# Threat model inicial (biblioteca embebida)

Estado: borrador para revisión · Relacionado con: [SAD §11](../architecture/SAD.md#11-seguridad-privacidad-y-ciclo-de-vida), ADR-0005, ADR-0006, ADR-0009, invariantes I-08, I-27…I-29

Este documento delimita **qué protege y qué no protege** Kerno en su forma inicial (biblioteca en el mismo proceso + CLI local). Debe revisarse antes de congelar el formato físico (ADR-0003) y de nuevo antes de cualquier servidor de red.

## 1. Alcance y supuestos

- Kerno se ejecuta **dentro** del proceso de la aplicación anfitriona, con sus mismos privilegios. Todo código de ese proceso puede leer la memoria de Kerno, incluidos datos descifrados y claves cargadas.
- La aplicación es responsable de autenticar a sus usuarios y de autorizar cada operación de negocio (SAD §11.1). Kerno ofrece mecanismos (límites, validación, identificadores opacos, auditoría), no decide quién puede hacer qué.
- El sistema operativo y el sistema de archivos son de confianza para el aislamiento entre usuarios del SO; no lo son para la integridad de los datos en reposo (pueden corromperse, truncarse o perder escrituras no sincronizadas).

## 2. Activos

| Activo | Por qué importa |
|---|---|
| Datos confirmados (estado actual) | Pérdida o alteración silenciosa es el fallo más grave del motor. |
| Historial de revisiones y ChangeSets | Contiene versiones antiguas, posiblemente datos personales ya «borrados» del estado actual. |
| Journal/WAL, páginas liberadas, segmentos obsoletos, temporales | Contienen copias de datos que el usuario cree borrados. |
| Backups y exportaciones | Viven fuera del control del motor y con otro ciclo de vida. |
| Claves de cifrado y de tokens | Su pérdida implica pérdida de datos; su robo, exposición de todo lo cifrado. |
| Identificadores internos (UUIDv7) | Revelan instante de creación y facilitan enumeración. |
| Logs, métricas y auditoría | Pueden filtrar contenido si no se minimizan. |

## 3. Adversarios y fallos considerados

| ID | Fuente | Ejemplos | Respuesta prevista |
|---|---|---|---|
| T-01 | **Fallos no maliciosos del medio** | bit rot, sectores dañados, escrituras rotas, truncamientos, `fsync` que falla | Checksums en todo lo persistido, creación atómica de archivos, detección y error explícito (I-08, I-09); envenenamiento tras `fsync` fallido (I-10). **Verificado en M0** con `SimVfs`. |
| T-02 | **Caída del proceso o del sistema** | kill -9, OOM, corte de energía durante commit, compactación o migración | Protocolo de commit y recuperación con matriz de fallos (ADR-0002); recuperación reentrante (I-11). **Verificado en M0.** |
| T-03 | **Acceso de lectura al disco o a backups** | portátil robado, backup en la nube, disco desechado | Cifrado en reposo con AEAD y claves fuera del directorio de datos (ADR-0006, I-28). *Pendiente (Fase 4).* |
| T-04 | **Manipulación deliberada de archivos** | un atacante edita páginas y recalcula CRC32C | CRC32C **no** protege frente a esto (R-S5). Requiere AEAD por página/bloque (ADR-0006). *Pendiente.* |
| T-05 | **Entradas hostiles vía API** | claves/valores enormes, rangos patológicos, documentos maliciosos | Límites explícitos y validación previa (I-13), límites de consulta y paginación (SAD §9.3), fuzzing (Fase 6). |
| T-06 | **Archivos de base hostiles** | abrir una base fabricada para provocar pánicos, bucles o lecturas fuera de rango | Validación de todas las longitudes, punteros y cadenas con límites; fuzzing del formato (Fase 6). Los prototipos ya acotan cadenas de overflow y longitudes. |
| T-07 | **Usuarios finales de la aplicación** | adivinar o enumerar IDs, manipular tokens | Tokens opacos y validados (ADR-0005); un token válido no autoriza (I-27); la autorización es de la aplicación. |
| T-08 | **Fugas por observabilidad** | contenido en logs, errores o auditoría | Auditoría minimizada y sin payloads (I-29); mensajes de error sin contenido de documentos. |
| T-09 | **Retención involuntaria** | revisiones, páginas liberadas, segmentos o backups que conservan datos suprimidos | Políticas de retención y supresión que abarcan derivados (ADR-0009); borrado criptográfico donde aplique. |
| T-10 | **Cadena de suministro** | dependencia comprometida | Pocas dependencias en el núcleo, `Cargo.lock` versionado, auditoría de dependencias (`cargo audit`/`cargo deny`) en Fase 6. |

## 4. Fuera de alcance (explícitamente)

- Atacantes con ejecución de código en el proceso anfitrión o con privilegios de administrador en la máquina mientras la base está abierta.
- Canales laterales (tiempos, caché de CPU) en las operaciones del motor.
- Garantizar el borrado físico en medios con wear-leveling o en copias que Kerno no controla (backups externos, snapshots del SO o de la nube); se documentan las ventanas, no se prometen.
- Cumplimiento normativo automático (SAD §11.5): los perfiles mapean controles, no certifican.

## 5. Consecuencias para M0 y siguientes fases

- La elección de almacenamiento (ADR-0001) afecta a T-03, T-04 y T-09: dónde quedan copias antiguas de los datos y qué unidad se cifra y autentica. Esto se recoge en ADR-0001 y ADR-0009.
- El formato (ADR-0003) debe reservar desde la primera versión espacio y versión para cifrado autenticado, aunque la Fase 1 no lo active.
- Ningún componente del núcleo requiere red ni proveedor externo (I-31).
