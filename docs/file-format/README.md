# Formato físico

Estado: **no congelado.** Ninguna base de datos de usuario debe crearse con los formatos de los prototipos de M0.

El formato del motor se define en [ADR-0003](../adr/0003-formato-fisico-y-versionado.md) una vez aceptado [ADR-0001](../adr/0001-motor-de-almacenamiento.md). Hasta entonces, este directorio contiene solo los principios que cualquier formato de Kerno debe cumplir, extraídos de SAD §13 y de lo aprendido en M0.

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
