# Etapa 9: auditoría minimizada (E9-03)

| Campo | Valor |
|---|---|
| Paquete | E9-03 de la [acta de la Etapa 9](etapa9-acta.md) (§3) |
| Invariantes | I-29 (extendida a `kerno-units`), I-37 |
| Hallazgos que cierra | H-6 y H-8 del [cierre del adelanto](etapa9-adelanto-cierre.md) |
| Decisión aplicada | D8: ningún registro persistido de auditoría; solo se verifica el texto que la capa ya emite |

## 1. Qué se verifica

`kerno-storage` y `kerno-units` no tienen capa de logs ni de auditoría. Lo que emiten son tres cosas, y las tres se buscan:

1. **Errores:** `Display` y `Debug` de cada variante que la prueba consigue producir.
2. **Informes:** `UnitVerifyReport`, `VerifyFinding` y el error que da `VerifyFinding::to_error`, con cada daño provocado por separado; el informe de retención.
3. **Observaciones:** `ObservationStats`, los registros del observador y las explicaciones, en los cuatro niveles.

Lo que devuelve una lectura correcta es el contenido que el llamador pidió y no se busca: lleva centinelas a propósito.

## 2. Pruebas

| Fichero | Qué hace |
|---|---|
| `crates/kerno-units/tests/redaction_units.rs` (nuevo) | Mismo detector que `kerno-storage/tests/redaction.rs`: cuatro centinelas (dos texto, dos binarios no UTF-8) en tipo, metadatos, carga, actor, fuente, motivo y correlación; formas texto, hexadecimal, lista decimal y `\x`; fragmentos de 8 bytes o más. Provoca ocho clases de error, seis daños distintos de registros y los cuatro niveles de observación, con `emulated_log` y con `kerno_db`. Falla si una clase de error, un hallazgo o un registro de observación no se produjo |
| `crates/kerno-storage/tests/redaction.rs` (solo la búsqueda de fragmentos) | `find_sentinels` busca además cada ventana de 8 bytes de cada centinela en cada forma (H-6). Las formas de texto de una ventana que no es UTF-8 se omiten: serían caracteres de sustitución |
| `crates/kerno-storage/tests/group_commit.rs` (solo el comentario) | H-8: el comentario decía «10^4 commits»; son 4 × 1 500 = 6 000 y 4 × 1 000 = 4 000 en las dos vueltas |

El detector de cada fichero tiene su propia prueba: encuentra cada forma y cada fragmento, no encuentra un texto limpio y no encuentra 7 bytes de un centinela (uno menos que el fragmento).

## 3. Mutaciones

Aplicadas a una copia del árbol (sin `target` ni `.git`), una cada vez, en modo release, y la copia se borró después. Salida: línea `FAILED` de cada prueba.

| # | Fichero y línea | Cambio | Resultado |
|---|---|---|---|
| M1 | `crates/kerno-units/src/db.rs:226` | el mensaje de «actor de N bytes excede el máximo» añade `&v[..10]` (el centinela binario entero) | `emulated_log::units_leak_no_content_in_errors_reports_or_observations` y `kerno_db::…` en `FAILED`; el detector sigue en verde |
| M2 | `crates/kerno-units/src/db.rs:226` | igual, con `&v[2..10]` (un fragmento de 8 bytes) | las dos pruebas por motor en `FAILED` («3 of 252 texts carry a sentinel (I-29)»); el detector sigue en verde |
| M3 | `crates/kerno-storage/src/sst.rs:335` | `Table::open` añade los 12 primeros bytes de `meta.smallest` al mensaje (un fragmento, no la clave entera) | las cinco pruebas de configuración en `FAILED` («N of ~18 000 texts carry a sentinel (I-29)»); el detector sigue en verde. Con la búsqueda anterior (clave completa) esta mutación habría pasado: es la prueba de H-6 |
| M4 | `crates/kerno-storage/tests/redaction.rs:118` | `windows(FRAGMENT)` pasa a `.take(0)` (el detector no busca fragmentos) | `detector_finds_every_form_of_every_sentinel` en `FAILED` |
| M5 | `crates/kerno-units/tests/redaction_units.rs:92` | igual que M4 | `detector_finds_every_form_and_fragment_of_every_sentinel` en `FAILED` |

## 4. Límites

- Los informes de `verify_units` y las observaciones no tienen mutación propia que filtre un centinela: el texto de `VerifyFinding` solo lleva claves de registro (identificadores), no contenido, así que no hay línea de producción que la prueba pueda ver fallar sin inventar una fuga. Lo que la prueba comprueba para esas rutas es que se ejercitaron (hallazgos y registros distintos de cero) y que ningún texto lleva un centinela.
- Un fragmento de menos de 8 bytes no se busca: una ventana más corta produce coincidencias con texto legítimo (números, nombres de regla).
- Los centinelas son los que la prueba planta. Un contenido que no pase por ellos no se ve.
- Esto no es un registro de auditoría (D8).
