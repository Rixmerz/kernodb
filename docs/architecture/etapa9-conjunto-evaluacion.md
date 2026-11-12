# Conjunto de evaluación de E8-10, congelación (E9-19)

| Campo | Valor |
|---|---|
| Estado | **Congelado.** Sin medir: la única cifra calculada es la validación de no saturación (§3) |
| Paquete | E9-19 de [etapa9-acta.md](etapa9-acta.md) §3 |
| Umbrales | T-E10-V1, T-E10-V2 y T-E10-R de §5 del acta, sin cambios (D22) |
| Medición | E9-20, una sola vez, sobre estos ficheros |

## 1. Qué es

Un conjunto nuevo, con semilla propia, para medir E8-10. Se construye sobre la misma historia sintética del conjunto de la Etapa 8 (`derived_eval`), con una diferencia: **todas las consultas llevan vector de consulta, en las cuatro clases**. En el conjunto de la Etapa 8 solo lo llevaba la clase de vecindad vectorial, y por eso Q4 no se podía calcular por clase (F-06).

| Elemento | Valor |
|---|---|
| Módulo | `crates/kerno-testkit/src/derived_eval2.rs` |
| Semilla | `20261203` (distinta de la de la Etapa 7 y de la de la Etapa 8) |
| Escala | calidad: 2 000 unidades, 300 cambios, 100 consultas |
| Consultas por clase | 25 en cada una de las cuatro clases, las 100 con vector |
| Vector de consulta | Vecindad vectorial: cerca de la base de la decisión semilla, como en la Etapa 8. Las demás clases: el vector de la unidad semilla con ruido de ±150 por componente |
| Juicios | Los grados 1 a 3 salen de los enlaces verdaderos de la historia, no de ningún motor. Para cada vector añadido se juzgan con grado 0 las diez unidades más cercanas por coseno que la caminata no juzgó, de modo que el top 10 exacto de cada consulta está juzgado por completo |

El conjunto no mide la calidad de ningún modelo: mide mecánica. Los vectores salen de una función de la semilla.

## 2. Ficheros congelados

`crates/kerno-testkit/data/derived-eval-2/`, escritos por `cargo run --release -p kerno-testkit --example freeze_derived_eval2`. Volver a ejecutarlo no cambia nada, y `tests/derived_eval2.rs` lo comprueba.

```text
4e599e109656f32a1d63c6b720c47942d23a11a28808c4bca3af275a9ae84c34  corpus.txt
ab84850aeaa94cf57ebefbd7e6b9626b79c41a308693c29508eb1e7547fab25a  vectors.txt
aea4e1208266c1fefb6cfb8e84718a6748488493cad44ad22f632e25afb25d84  queries.txt
```

Este es el contenido de `MANIFEST` (T-E10-R). Antes de medir, E9-20 comprueba que coincide; si no, no se mide.

## 3. Validación de no saturación (T-E10-V2)

El brazo sin señal ordena por el vector de consulta solo (búsqueda exacta por coseno). Su nDCG@10 en conjunto debe ser ≤ 0,96, o el conjunto se regenera con otra semilla **antes** de congelarlo. Se calculó una vez, con la primera semilla:

| Clase | nDCG@10 sin señal |
|---|---|
| vecindad vectorial | 0,7018 |
| temporal | 0,3483 |
| procedencia | 0,2674 |
| combinada | 0,3687 |
| **en conjunto** | **0,4216** |

0,4216 ≤ 0,96, así que no hubo regeneración y la semilla es la primera que se probó. Esta es la única cifra de calidad que se calcula con el conjunto; no se mide el brazo con señal ni ningún otro. El ejemplo `freeze_derived_eval2` repite la comprobación y no escribe nada si se supera el límite, y la prueba `arm_without_signal_is_not_saturated` la repite sobre los ficheros congelados.

## 4. Pruebas

`crates/kerno-testkit/tests/derived_eval2.rs`: 8 pruebas (el conteo de `kerno-testkit` pasa de 242 a 250). Ninguna abre una base de datos, así que no hay motor que variar. Cada una se vio fallar con una mutación sobre una copia del árbol (semilla, vector omitido, límite de saturación, juicios de vecinos, byte de un fichero, vector quitado de una consulta, juicios de grado 0 quitados, dependencia añadida).

## 5. Qué sigue

E9-20 mide sobre estos ficheros, una sola vez. Ningún fichero de este paquete ni ningún umbral se cambia después de medir.
