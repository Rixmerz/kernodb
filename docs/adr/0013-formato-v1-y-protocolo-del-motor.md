# ADR-0013 — Formato v1 y protocolo del motor

| Campo | Valor |
|---|---|
| Estado | Propuesto (redactado en WP06; su aceptación por el propietario es requisito de la congelación del formato en WP14 y no bloquea otros paquetes) |
| Fecha | 2026-10-28 |
| Decisores | Propietario del proyecto (pendiente) |
| Relacionado con | ADR-0001, ADR-0002, ADR-0003 (aceptados, que este ADR interpreta sin reemplazar), ADR-0006, ADR-0012; [formato v1](../file-format/v1.md); [diseño de la Fase 1](../architecture/phase1-design.md) §4, §5, §7, §12 y §13; invariantes I-03, I-08, I-09, I-11, I-15, I-16, I-18 |

## Contexto

ADR-0001, ADR-0002 y ADR-0003 fijan **qué** debe cumplir el núcleo (LSM nivelado, semántica de commit y durabilidad, versionado e integridad), pero dejan abiertos los detalles de **cómo** se concretan en bytes y en el protocolo del motor. El diseño de la Fase 1 (C-01…C-33) y el borrador [`v1.md`](../file-format/v1.md) toman esas decisiones; algunas interpretan una regla aceptada de una forma que el propietario debe poder revisar antes de que el formato se congele, porque después cambiarlas exige subir `format_version` o un bit de característica (v1.md §15).

Este ADR reúne esas interpretaciones, con las alternativas que se descartaron y el motivo. No cambia ningún ADR aceptado: donde un ADR aceptado necesita una nota de seguimiento, se añade allí (C-21).

Desde WP06 el formato existe como código puro (`crates/kerno-storage/src/format/`), con 194 vectores de estructura (`crates/kerno-storage/tests/vectors/v1/structs/`), un decodificador independiente escrito desde la especificación (`tests/spec_decoder/`) y bloques hexadecimales en v1.md §13.5 que la CI compara con los vectores. El motor todavía no lo usa (WP07).

## Opciones consideradas

Cada punto indica la opción elegida (**en negrita**) y las descartadas.

1. **Cabecera común con `db_id`** (ADR-0003 §1).
   - **Cabecera de 96 bytes en el desplazamiento 0 de los tres tipos de archivo, con `db_id` aleatorio de 16 bytes y el número de archivo, verificados contra el MANIFEST y el nombre (HDR-10).** Detecta archivos de otra base o renombrados con un CRC válido, que ningún checksum detecta.
   - Cabecera al final en los archivos de solo anexado (ADR-0003 §1 lo permite): un log no puede terminar en una cabecera mientras se escribe, y la cabecera al principio permite rechazar por versión antes de leer nada más.
   - Sin `db_id` (solo *magic* y versión): una SST copiada de otra base con el mismo número se aceptaría.
2. **Sin archivo `CURRENT`.**
   - **`MANIFEST` con nombre fijo, reemplazado por rename atómico en cada *checkpoint* (W-10).** Un solo archivo define el estado; la creación (DIR-3) y el reemplazo son el mismo protocolo temporal → `fsync` → `rename` → `fsync` del directorio de ADR-0003 §3.
   - `CURRENT` que apunta a `MANIFEST-NNNNNN` (estilo LevelDB/RocksDB): un archivo y una sincronización más por *checkpoint*, y un estado intermedio más que recuperar.
3. **`CLEAN` dentro del *checkpoint* y `SESSION_OPEN`** (ADR-0003 §3, ADR-0001 condición 3).
   - **El cierre limpio escribe un *checkpoint* cuyo único registro es un `SNAPSHOT` con `CLEAN`; la reapertura anexa `SESSION_OPEN`, que lo anula (MAN-R4).** Tras un cierre limpio el directorio contiene solo `LOCK`, `MANIFEST` (un registro) y SST, así que ningún daño posterior es ambiguo; una reapertura limpia cuesta 2 sincronizaciones (M0: 3).
   - Un registro `CLEAN` anexado al log: deja un log largo tras el cierre y obliga a decidir si una cola rota después de `CLEAN` es daño.
   - Un archivo marcador aparte: otro archivo cuyo estado hay que hacer atómico con el MANIFEST.
4. **`synced_len` en lugar del bit `SYNCED`** (ADR-0002, C-04).
   - **Cada trama del WAL lleva `synced_len`, los bytes del archivo que el escritor sabía duraderos al escribirla (W-11); WAL-R5 distingue cola rota de daño buscando una trama posterior con `synced_len` más allá del punto roto.** Funciona igual con un escritor en `Sync`, con *group commit* (una trama = un grupo, un CRC) y en `Buffered`, y admite durabilidad por commit sin cambiar el formato. Para que valga con grupos, la cota de `first_seq` de una trama candidata en `q` cuenta un commit por byte desde el punto roto (`q − P`), no un commit por trama como en el M0: una trama puede llevar hasta 65 536 commits de un solo byte (WAL-C3), y con la cota del M0 un daño en un grupo duradero se tomaba por cola rota (corregido en la revisión de WP06; lo fijan los vectores `neg_walr5_group_*` y `neg_walr6_group_broken_header`).
   - El bit 31 `SYNCED` del M0 por registro: no expresa grupos ni el prefijo sincronizado en `Buffered`.
   - Aceptar cualquier cola rota del WAL más reciente: pierde en silencio commits confirmados si el daño está a mitad del archivo (el defecto medio-alto del M0).
5. **Ranura AEAD** (ADR-0003 §2, ADR-0006).
   - **Ranura de 40 bytes (`nonce[24] ‖ tag[16]`) siempre presente en cada bloque de SST y cada registro de MANIFEST; en las tramas del WAL, solo si la cabecera de ese WAL tiene el bit `AEAD` (por archivo). Con `AEAD` desactivado la ranura debe ser cero y la cubre el CRC (AEAD-1).** Un WAL nunca sobrevive a un cierre limpio, así que activar el cifrado no exige migrarlo, y los commits pequeños no pagan 40 bytes por trama (T7).
   - Ranura siempre presente también en el WAL: una sola constante, `FRAME_SLOT_ALWAYS` (`format::features`), y la especificación (WAL-R6, v1.md §5) cubren ya esta variante; los vectores incluyen una trama con ranura (`frame_with_slot`).
   - Ranura solo con el bit activado en todos los tipos: activar el cifrado obligaría a reescribir todas las SST y el MANIFEST.
6. **Lápidas de rango en la base de v1** (ADR-0001 condición 2, C-11).
   - **`DELETE_RANGE` en el commit canónico, bloque de rangos fragmentados en la SST y `largest_exclusive` en `FileMeta`, sin bit `incompat`.** Todo lector v1 los entiende desde el primer día; las SST de solo rangos son válidas.
   - Añadirlas después con un bit `incompat`: cada base creada antes quedaría sin poder suprimir rangos sin migración.
7. **Una versión por clave de usuario en cada SST** (SST-I1).
   - **Cada SST contiene como mucho una versión por clave; las instantáneas retienen archivos completos (W-6, SEM-5).** El lector no necesita fusionar versiones dentro de una tabla y la visibilidad se decide solo por secuencias.
   - SST multiversión: llegarían con un bit `incompat` si la retención de instantáneas largas lo exigiera.
8. **Errores `Incompatible` y `ReadOnly`** (ADR-0002, C-16).
   - **Dos variantes nuevas de `kerno_core::Error`, que se marca `#[non_exhaustive]`.** Una versión o característica desconocida no es daño y no debe presentarse como `Corruption`; la CRC de la cabecera se comprueba antes que la versión (HDR-3 antes de HDR-4) para que el daño aleatorio nunca parezca `Incompatible`.
   - Reutilizar `Corruption` o `InvalidArgument`: confundiría a quien decide si restaurar una copia o actualizar el binario.
9. **Solo lectura automática ante un bit `ro_compat` desconocido** (ADR-0003 §1, HDR-7).
   - **La base entera se abre en solo lectura y `is_read_only()` informa del archivo y los bits.** Es la semántica de ADR-0003 llevada al nivel de la base: un archivo con un bit `ro_compat` desconocido hace que ningún escritor v1 pueda tocar el conjunto.
   - Rechazar con `Incompatible`: impide leer datos que un lector v1 entiende.
   - Abrir en lectura-escritura por archivo: un escritor reescribiría ese archivo en una compactación sin conservar lo que no entiende.
10. **Bit `DRAFT` y proceso de congelación** (ADR-0003, v1.md §4.3 y §15).
    - **Hasta la congelación todo archivo lleva `incompat` bit 63; al congelar (WP14) el bit sale del conjunto conocido y una base del borrador se rechaza con `Incompatible("pre-freeze draft format")`.** Ninguna base creada con reglas que pudieron cambiar se lee con las reglas definitivas. Mientras rige, cambiar un byte exige historial en v1.md, vectores regenerados con nota en este ADR y `spec_decoder_agrees`.
    - Sin marca: una base del borrador podría abrirse tras la congelación y leerse mal.
11. **Límites de formato frente a límites de API** (v1.md §1.4, C-26).
    - **Los lectores aplican límites de formato amplios (`Corruption`); el escritor aplica límites de API más estrictos (`InvalidArgument`) antes de escribir nada.** Subir un límite de API dentro del de formato no es un cambio de formato.
    - Un único límite: cualquier ampliación futura sería un cambio de formato.
12. **Durabilidad por commit diferida** (ADR-0002, C-05).
    - **La Fase 1 ofrece durabilidad por base (`Sync`/`Buffered`) y `sync()`; `commit_with` queda para después.** El formato ya la admite: `synced_len` describe cualquier mezcla de tramas sincronizadas y no sincronizadas.
    - Implementarla ya: más estados que probar en los barridos de caídas sin un caso de uso que la pida en la Fase 1.
13. **Plataforma verificada: solo Linux** (ADR-0002, C-28).
    - **Las garantías de `Sync` se verifican en Linux (ext4/xfs); en otra plataforma, abrir con `Sync` exige `allow_unverified_platform`.** macOS (`F_FULLFSYNC`) y Windows quedan sin verificar.
    - Prometer las mismas garantías en todas: no hay evidencia que las respalde (C-28).
14. **Objetivos de rendimiento T1–T10** (ADR-0001 condición 1, ADR-0012).
    - **Los objetivos son los de [phase1-design.md](../architecture/phase1-design.md) §7.2, registrados antes de medir (WP04) y aprobados por el propietario (C-06).** Este ADR los cita por referencia: no los duplica para que no diverjan.

15. **Preasignación y reciclado del WAL sin bit `incompat`** (seguimiento de WP05, ADR-0001 condición 1).
    - **El escritor deja el WAL como `cabecera ‖ ceros` y reutiliza un WAL ya volcado tras sobrescribirlo entero con ceros; el lector no cambia.** Es el relleno de ceros que §6.1 y WAL-R4 ya admiten, así que ningún archivo válido cambia de bytes y los vectores siguen idénticos (v1.md §6.5, nota «Preasignación y reciclado»). Las tramas de la vida anterior de un archivo reciclado no se reproducirían (su CRC cubre otro número de archivo), pero un WAL intermedio de la cadena que termine en ellas sería una cola rota (REC-10a): por eso se sobrescriben con ceros antes de renombrar el archivo a su nombre canónico.
    - Reciclado sin poner a cero, con el bit `incompat` reservado: ahorra la escritura de ceros, pero exige que el lector acepte como final limpio un archivo que acaba en tramas ajenas (un cambio de WAL-R4 y de REC-10a que invalida el barrido de caídas y los vectores). Se deja para cuando la medición pida ese ahorro.

## Criterios de decisión

Fijados antes de implementar el códec:

- Ninguna regla aceptada de ADR-0001/0002/0003 se relaja: todo byte que influye en un resultado está cubierto por un CRC (I-08) y por una regla de validación con identificador.
- Un daño aleatorio nunca se presenta como `Incompatible`; una incompatibilidad nunca se presenta como daño.
- Lo que una caída deja a medias se distingue de un daño posterior sin heurísticas dependientes del tiempo (I-03, I-11).
- Coste en el camino caliente: ningún byte por commit que no sea necesario (T7) y ninguna sincronización extra en la reapertura limpia (T8).
- Cada decisión se puede revertir antes de la congelación cambiando el borrador y regenerando vectores (v1.md §15).

## Evidencia

- El códec (`crates/kerno-storage/src/format/`) implementa v1.md §1–§8 y §12 como funciones puras, con 194 vectores (45 positivos, 149 negativos que cubren cada regla que un decodificador puede informar, con su clase).
- `spec_decoder_agrees`: un decodificador escrito solo desde v1.md coincide con el códec en regla, clase y contenido en todos los vectores y en 5 000 entradas editadas por decodificador en cada ejecución (1,4 millones en una ejecución local con `KERNO_AGREE_INPUTS=100000`), incluidas ediciones dentro de un bloque, una trama o un registro con su CRC recalculado.
- `decoders_never_panic_on_random_bytes`: 10^5 entradas por cada uno de los 14 decodificadores sin pánicos.
- La función bloom es la del M0 (`bloom_matches_m0_function`), y la forma canónica del commit equivale a aplicar el lote operación a operación (`canonicalize_matches_sequential_application`, aleatorizada).
- La evidencia de comportamiento del motor (barridos de caídas, corrupción a 1000 semillas, `kill -9`, LazyFS) llega con WP07–WP14 y se enlazará aquí antes de pedir la aceptación.

## Decisión

Se adoptan las opciones marcadas en los puntos 1–14 para el formato v1 y el protocolo del motor de la Fase 1, tal como las especifica [`v1.md`](../file-format/v1.md):

| # | Decisión | Interpreta | Especificación |
|---|---|---|---|
| 1 | Cabecera común de 96 bytes con `db_id` y número de archivo | ADR-0003 §1 | v1.md §3, HDR-10 |
| 2 | Sin `CURRENT`; `MANIFEST` reemplazado por rename | ADR-0003 §3 | v1.md §2, §8.9 |
| 3 | `CLEAN` en el *checkpoint*; `SESSION_OPEN` al reabrir | ADR-0003 §3 | v1.md §8.5, §9.4, MAN-R4 |
| 4 | `synced_len` por trama | ADR-0002 | v1.md §6.2, W-11, WAL-R5 |
| 5 | Ranura AEAD por bloque y registro; en el WAL, por archivo (`FRAME_SLOT_ALWAYS = false`) | ADR-0003 §2 | v1.md §5 |
| 6 | Lápidas de rango en la base | ADR-0001 cond. 2 | v1.md §6.3, §7.4 |
| 7 | Una versión por clave en cada SST | ADR-0001 | v1.md SST-I1 |
| 8 | `Incompatible` y `ReadOnly` | ADR-0002 | phase1-design §4 |
| 9 | Solo lectura ante `ro_compat` desconocido | ADR-0003 §1 | v1.md HDR-7 |
| 10 | Bit `DRAFT` y congelación | ADR-0003 | v1.md §4.3, §15 |
| 11 | Límites de formato y de API separados | ADR-0003 §2 | v1.md §1.4 |
| 12 | Durabilidad por commit diferida | ADR-0002 | phase1-design §9 |
| 13 | Solo Linux verificado | ADR-0002 | phase1-design §5.10 |
| 14 | T1–T10 por referencia | ADR-0001, ADR-0012 | phase1-design §7.2 |

## Consecuencias

- **Formato:** v1 queda definido byte a byte con identificadores de regla; cualquier cambio posterior a la congelación exige `format_version` o un bit nuevo, vectores y un ADR.
- **Recuperación:** la tabla de decisión (v1.md §9) forma parte del formato; WP08 la implementa como función pura compartida por la apertura, la solo lectura y `verify()`.
- **Cifrado:** activar AEAD (ADR-0006) no exigirá migrar SST ni MANIFEST; los WAL se crean ya con el bit.
- **Coste:** 32 bytes de cabecera de trama por grupo de commits y 44 por bloque de SST; una reapertura limpia, 2 sincronizaciones.
- **`synced_len` y cabecera rota (punto 4):** la primera trama de cada WAL lleva `synced_len = 0`, porque W-1 no sincroniza la cabecera. Si tras una caída la cabecera del WAL más reciente se daña (no por la caída, sino por un fallo del medio) y ese WAL solo tiene tramas con `synced_len = 0`, WAL-R6 lo trata como un WAL sin tramas, y un commit confirmado en `Sync` puede perderse sin error. Es un caso de daño posterior a la escritura, no de caída; las suites de corrupción del M0 actúan tras un cierre limpio, cuando no queda ningún WAL. La alternativa es sincronizar la cabecera en W-1 (un `sync_data` más por rotación, una vez cada `write_buffer_bytes`), con lo que toda trama llevaría `synced_len ≥ 96` y WAL-R6 podría rechazar cualquier trama válida bajo una cabecera rota. Queda como decisión del propietario (ver Seguimiento).
- **Precisiones de WP06** (historial de v1.md, sin cambiar ningún archivo válido): la cabecera rota de WAL-R6 incluye ahora HDR-2 con un *magic* desconocido (ni de v1 ni del M0), porque una cabecera nunca sincronizada puede quedar a ceros mientras la primera trama sí llega al disco (el modelo de `SimVfs` lo produce); la tabla de §13.1 fija qué regla informa cada comprobación y en qué orden, que es lo que permite que dos decodificadores independientes coincidan. La revisión de WP06 corrigió la cota de WAL-R5 y WAL-R6 para tramas de grupo (punto 4) y añadió al lector dos límites de §1.4 que no aplicaba (`last_seq` y `edit_seq` ≤ 2^63−1, para que ninguna secuencia derivada desborde); ningún vector existente cambió.

## Seguimiento

- Antes de la congelación (WP14): aceptación por el propietario, en particular de los puntos 2, 3, 4, 5, 8, 9 y 10 (phase1-design §13, decisión 2), y revisión del modelo de amenazas (R-07).
- Decidir si W-1 sincroniza la cabecera del WAL (consecuencia del punto 4) antes de WP08, que implementa el protocolo de sesión; si se adopta, WAL-R6 se endurece en el borrador con su entrada de historial.
- Enlazar aquí la evidencia del motor (WP07–WP14) y la medición de T7 y T8.
- Registro de cambios de bytes del borrador (v1.md §15): ninguno; los vectores de WP06 son los primeros.
