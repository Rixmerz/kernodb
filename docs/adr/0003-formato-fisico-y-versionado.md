# ADR-0003 — Formato físico y versionado

| Campo | Valor |
|---|---|
| Estado | **Aceptado** (2026-10-28, por el propietario del proyecto): las reglas de versionado e integridad son obligatorias; el formato concreto se congela al cerrar Fase 1 |
| Fecha | 2026-10-27 (propuesta) · 2026-10-28 (aceptación) |
| Relacionado con | SAD §13, D-10; ADR-0001, ADR-0002, ADR-0006; [principios de formato](../file-format/README.md); invariantes I-08, I-09, I-15, I-30 |

## Contexto

Una vez existan bases de datos reales, cambiar el formato será una migración, no una refactorización. SAD §13 pide tres ejes de versión separados (formato físico, esquema lógico, datos derivados), rechazo seguro de versiones incompatibles, detección de truncamientos y corrupción, y una exportación lógica independiente.

M0 no congela ningún formato: los dos prototipos tienen formatos propios, versionados solo por *magic* y un número de versión. Lo que sí deja M0 es evidencia de **qué propiedades de formato hacen falta** para pasar las pruebas de integridad y de fallos, y eso es lo que fija este ADR.

## Decisión

### 1. Identificación y versión de cada archivo

Todo archivo persistente empieza (o, si es de solo-anexado, termina) con una cabecera fija:

| Campo | Contenido |
|---|---|
| `magic` | 8 bytes por tipo de archivo (`KRNO…`), distintos para datos, journal, manifest, exportación |
| `format_version` | `u32` del formato físico de ese tipo de archivo |
| `compat` / `ro_compat` / `incompat` | Tres conjuntos de bits de características, al estilo ext4: un lector que no conoce un bit `incompat` **rechaza** abrir; uno que no conoce un bit `ro_compat` abre **solo en lectura**; los bits `compat` se ignoran sin riesgo |
| `page_size` / `block_size` | si aplica |
| `crc32c` de la cabecera | |

Así, una función nueva (p. ej. cifrado, compresión, un índice nuevo) se añade como bit de característica sin forzar una migración completa, y una versión antigua de Kerno nunca escribe sobre un formato que no entiende.

### 2. Integridad

- **Todo byte que influya en un resultado está cubierto por un checksum que se verifica antes de usarlo** (I-08). Las pruebas de M0 mostraron que cualquier excepción —un bloom filter sin crc, una lista libre sin crc— produce resultados erróneos silenciosos.
- Se conserva el modelo de M0 según el motor elegido: checksums en el padre (árbol Merkle) para un CoW B+ tree; checksum por registro/bloque/archivo para un LSM.
- **Checksum de cabecera propio en cada página/bloque** además del del padre, para poder verificar y depurar (`kerno verify`) páginas sueltas sin recorrer el árbol.
- CRC32C para detectar daño accidental. La integridad frente a manipulación es AEAD (ADR-0006): el formato reserva desde la versión 1 espacio por página/bloque para nonce y etiqueta, y un bit `incompat` indica si están en uso.
- Toda longitud, desplazamiento, contador o puntero leído del disco se valida contra el tamaño del archivo y los límites del formato **antes** de indexar, reservar memoria o iterar; las cadenas (overflow, listas libres) se recorren con un límite derivado de datos verificados.

### 3. Atomicidad de los archivos que definen el estado

- Creación y reemplazo atómicos (temporal → `fsync` → `rename` → `fsync` del directorio).
- Un archivo existente vacío, corto o sin estado válido es `Corruption`, nunca «base nueva» (I-09). En M0 ambos prototipos tuvieron que adoptar esto para pasar la suite de truncamientos.
- Tras un **cierre limpio** no debe quedar ambigüedad entre «cola rota» y «corrupción»: o se marca el cierre limpio, o el estado queda en estructuras sin cola (checkpoint completo), o hay redundancia (dos copias del meta con el mismo commit).

### 4. Ejes de versión independientes (SAD §13)

| Eje | Dónde vive | Quién lo cambia |
|---|---|---|
| Formato físico | cabecera de cada archivo + bits de características | releases del motor |
| Esquema lógico | metadatos de colección (claves reservadas) | la aplicación (migraciones de esquema) |
| Datos derivados | definición de cada índice/embedding: algoritmo, versión, parámetros, watermark | Kerno al reconstruir (ADR-0007) |

Cambiar la versión de un índice derivado **no** requiere migrar el formato físico: el índice se marca `stale` y se reconstruye desde la fuente.

### 5. Migraciones del formato físico

- Precondiciones comprobadas (versión de origen soportada, espacio libre, backup reciente o confirmación explícita).
- La migración escribe estructuras nuevas sin modificar las de origen; el cambio de versión es un commit atómico más (meta o manifest), de modo que la matriz de fallos de ADR-0002 la cubre.
- El estado de la migración se persiste: tras una caída, la apertura reanuda o revierte.
- Validación posterior (`verify` completo) antes de borrar las estructuras de origen.
- Cada release documenta qué versiones abre directamente, cuáles migra y cuáles rechaza. No se promete compatibilidad hacia atrás indefinida ni *downgrade* automático.
- Las pruebas de migración se ejecutan con interrupción simulada en cada punto de E/S (misma técnica que `crash::every_point`).

### 6. Exportación lógica

Formato independiente del almacenamiento interno, versionado y documentado: una cabecera (versión, origen, secuencia de commit del snapshot exportado), seguida de registros de colección, documento, revisión, `ChangeSet` y mapeos de identificadores públicos, con checksum por registro y un registro final con recuentos. Codificación propuesta: JSON Lines para legibilidad y CBOR determinista (RFC 8949 §4.2) para fidelidad binaria; la elección final depende de ADR-0008. La exportación se toma siempre desde un snapshot (consistente sin bloquear escritores).

### 7. Vectores de prueba

Cada versión de formato publica archivos de ejemplo y sus contenidos esperados. Las pruebas de compatibilidad abren todos los vectores de versiones soportadas y comprueban el rechazo seguro de las no soportadas y de bits `incompat` desconocidos.

## Evidencia de M0

- Bit flips (400 semillas, la mitad dirigidos a cabeceras, pies y metas) y truncamientos (incluido a 0 bytes) tras cierre limpio: ambos prototipos devuelven datos correctos o error, nunca datos erróneos ni un commit anterior ([informe M0](../testing/m0-report.md)).
- Fallos reales que las pruebas obligaron a corregir y que motivan las reglas anteriores: corrupción a mitad del log tratada como cola rota tras un cierre limpio (regla 3, motor de referencia); sincronización del directorio solo al crear el archivo (regla 3 y ADR-0002; motor de referencia y B+ tree); elección del slot de meta por paridad de la secuencia, que tras perder un slot habría sobrescrito el único meta válido (regla 3; B+ tree).
- Riesgo de diseño señalado antes de implementar y cubierto por la regla 2: un filtro bloom dañado sin checksum haría que claves presentes parecieran ausentes, sin ningún error.

## Consecuencias

- El formato concreto (layout de páginas o bloques) se escribe en `docs/file-format/` durante Fase 1 a partir del prototipo elegido en ADR-0001, con estas reglas como requisitos.
- Ninguna base de datos de usuario se crea antes de que ese documento y sus vectores de prueba existan (SAD §20.6).
- Reservar espacio para AEAD desde la versión 1 cuesta unos bytes por página/bloque, pero evita una migración completa al activar el cifrado.

## Seguimiento

- Fase 1: especificación del formato v1, vectores de prueba, `kerno verify`.
- Fase 4: exportación lógica e importación, pruebas de ida y vuelta.
- Fase 6: fuzzing de lectores de formato; pruebas de compatibilidad entre releases.
