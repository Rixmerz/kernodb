# Formato físico

Estado: el formato del motor LSM está especificado en **[`v1.md`](v1.md)** y **congelado desde 2026-10-31** ([ADR-0013](../adr/0013-formato-v1-y-protocolo-del-motor.md) aceptado). El bit `incompat` `DRAFT` está retirado: `kerno-storage` ya no lo escribe y una base creada con el borrador se rechaza con `Incompatible("pre-freeze draft format")` (v1.md §4.3). Cambiar un byte exige `format_version` nuevo o un bit de característica, vectores y un ADR (v1.md §15).

Contenido del directorio:

| Archivo | Qué es |
|---|---|
| [`v1.md`](v1.md) | Especificación normativa del formato v1 (directorio, cabecera común, WAL, SST, MANIFEST, recuperación, semántica, vectores, compatibilidad, historial del borrador) |
| `crates/kerno-storage/tests/vectors/v1/` | Vectores de estructura y de directorio; una guardia de CI los regenera en memoria y los compara byte a byte ([CONTRIBUTING](../../CONTRIBUTING.md#vectores-del-formato-y-cambios-de-bytes)) |

Los principios siguientes se extrajeron de SAD §13 y de lo aprendido en M0 antes de escribir el formato; `v1.md` los cumple, y siguen siendo los requisitos de cualquier formato futuro de Kerno.

## Principios obligatorios

1. **Magic y versión en cada archivo persistente** (y en cada meta/cabecera de commit). Una versión desconocida se rechaza sin modificar el archivo.
2. **Longitudes explícitas y límites máximos** para toda estructura variable; ninguna longitud o puntero leído del disco se usa sin validar contra el tamaño del archivo y los límites del formato.
3. **Checksum sobre todo byte que influya en un resultado.** M0 usa CRC32C; la integridad criptográfica (AEAD) se decide en ADR-0006 y el formato debe reservar espacio y versión para ella.
4. **Commit identificable.** Debe poder determinarse sin ambigüedad cuál es el último commit válido, incluido tras escrituras rotas o reordenadas (ADR-0002).
5. **Creación atómica.** Los archivos que definen el estado (meta, manifest) se crean o reemplazan con *archivo temporal → fsync → rename → fsync del directorio*; un archivo existente vacío o corto es corrupción, no «base nueva».
6. **Distinción entre cola rota y corrupción.** Un registro inválido al final del journal tras una caída es una cola rota; tras un cierre limpio, o seguido de registros válidos, es corrupción y se reporta.
7. **Tres ejes de versión independientes:** formato físico, esquema lógico y datos derivados (SAD §13).
8. **Exportación lógica independiente** del formato físico, versionada (I-30).
9. **Migraciones** con precondiciones, ejecución reanudable o abortable, validación posterior y ruta de restauración desde backup; el estado de la migración se persiste.
10. **Vectores de prueba** versionados para cada estructura binaria (archivos de ejemplo + valores esperados), usados por las pruebas de compatibilidad.

## Referencias no normativas

- Formato del prototipo CoW B+ tree: [`spikes/storage/spike-btree/DESIGN.md`](../../spikes/storage/spike-btree/DESIGN.md)
- Formato del prototipo LSM: [`spikes/storage/spike-lsm/DESIGN.md`](../../spikes/storage/spike-lsm/DESIGN.md)
