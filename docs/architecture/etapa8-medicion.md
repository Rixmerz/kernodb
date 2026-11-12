# Etapa 8 · E8-07 — Medición de los derivados

| Campo | Valor |
|---|---|
| Fecha | 2026-11-12 |
| Estado | **Medido.** Calidad: Q1, Q2, Q3 y Q5 se cumplen; Q4 se cumple en lo que se puede calcular y **no se cierra**, porque 3 de sus 4 clases no son medibles con este conjunto. Q6 no se mide. Rendimiento: P1 a P4 se cumplen |
| Acta | [Etapa 8](etapa8-acta.md) §6.2 (calidad), §6.3 (rendimiento), decisiones D4, D5, D9 y D10 (§8, §8.1), fila E8-07 |
| Conjunto | `crates/kerno-testkit/data/derived-eval/` (`MANIFEST` verificado antes de medir) · [conjunto de evaluación](etapa8-conjunto-evaluacion.md) |
| Código | `crates/kerno-units/tests/derived_measure.rs` (calidad, P3 y P4; se ejecuta con `cargo test`) · `crates/kerno-units/examples/derived_bench.rs` (P1 y P2; no se ejecuta con `cargo test`) · `crates/kerno-units/tests/harness/derived.rs` (cargador del conjunto) |
| Salida cruda | [etapa8-medicion-salida.txt](etapa8-medicion-salida.txt) |

Este documento no cambia ningún umbral (D4), ninguna respuesta esperada ni ningún peso. Toda cifra sale de la salida cruda; lo que no se afirma está en §6.

## Resultado en una línea

Todos los umbrales de §6.2 y §6.3 que se pueden calcular se cumplen. **Pero el conjunto solo trae vector de consulta en 25 de sus 100 consultas** (las de la clase de vecindad vectorial), así que Q2, Q3 y Q4 se calculan sobre esas 25, y Q4 «en cada clase» no se puede calcular en las otras tres clases: se informa como **no medible**, no como cumplido.

## 1. Método

### 1.1 Comprobaciones previas a cualquier cifra

Se ejecutan dentro de la prueba; si una falla, la prueba falla antes de imprimir cifras.

| Comprobación | Resultado (salida cruda) |
|---|---|
| `MANIFEST`: SHA-256 de `corpus.txt`, `vectors.txt` y `queries.txt` | «match» en ambos motores. Si no coincide, la prueba se niega a medir (`a_manifest_that_does_not_match_refuses_to_measure`) |
| Carga | 2 000 unidades, 5 062 relaciones, 300 conjuntos de cambios; 2 000 vectores persistidos, reabiertos y servidos |
| Consultas | 100 traducidas y ejecutadas por la ruta de referencia, la indexada y el planificador; 0 caídas del planificador |

### 1.2 Brazos (corte en 10, sobre las 100 consultas salvo donde se dice)

- **simple**: orden de referencia «distancia, luego identificador». Los identificadores están permutados con semilla, así que el desempate no favorece a ninguna estructura.
- **ranked**: el mismo resultado con `RankMode::On`, pesos por defecto, sin vectores. Es el «sin señal» de Q4.
- **vector** (solo las 25 consultas con vector): búsqueda coseno exacta sobre los vectores persistidos y leídos de la base reabierta.
- **hybrid** (las mismas 25): `ranked` con la señal vectorial, sobre los vectores persistidos. Es el «con señal» de Q2, Q3 y Q4.
- **brute** (las mismas 25): orden coseno por fuerza bruta, independiente, sobre los vectores enteros del conjunto. Es la referencia de Q1.

Las unidades sin juzgar del top 10 se excluyen de la lista antes de calcular las métricas y se cuentan por brazo (acta §6.1). El ideal de nDCG y el total de relevantes de recall siguen siendo los de toda la lista de juicios.

### 1.3 Qué consultas llevan vector

| Clase | Consultas con vector |
|---|---|
| `vector_neighbourhood` | 25 de 25 |
| `temporal` | 0 de 25 |
| `provenance` | 0 de 25 |
| `combined` | 0 de 25 |

La señal vectorial solo se puede aplicar a una consulta que trae vector. Donde el umbral dice «en conjunto», la cifra es la de esas 25 consultas, y así se rotula en cada línea de la salida («computed on the 25 queries with a vector»). Donde dice «en cada clase», las tres clases sin vector no se pueden calcular.

## 2. Calidad (§6.2)

Valores idénticos en `emulated_log` y `kerno_db` (la salida compara las 63 líneas de cada motor y las da por idénticas).

| Umbral | Cota | Valor medido | Sobre cuántas consultas | Estado |
|---|---|---|---|---|
| **Q1** recall@10 de la búsqueda exacta persistida frente a fuerza bruta | = 1,000 | **1,000**; 25 de 25 listas idénticas y en el mismo orden | 25 | **cumple** |
| **Q2** nDCG@10 con señal (hybrid), en conjunto | ≥ 0,80 | **1,000** | 25 | **cumple** (sobre las 25) |
| **Q3** recall@10 con señal (hybrid), en conjunto | ≥ 0,90 | **1,000** | 25 | **cumple** (sobre las 25) |
| **Q4** nDCG@10 con señal ≥ sin señal − 0,01, en conjunto | ≥ 0,990 | 1,000 (sin señal 1,000) | 25 | **cumple** (sobre las 25) |
| **Q4** … `vector_neighbourhood` | ≥ 0,990 | 1,000 (sin señal 1,000) | 25 | **cumple** |
| **Q4** … `temporal` | — | — | 0 de 25 | **no medible** |
| **Q4** … `provenance` | — | — | 0 de 25 | **no medible** |
| **Q4** … `combined` | — | — | 0 de 25 | **no medible** |
| **Q5** sin juzgar ≤ 10 % del top 10, en cada brazo | ≤ 10 % | simple 0,0 % (0 de 708); ranked 1,1 % (8 de 708); vector 0,0 % (0 de 250); hybrid 0,0 % (0 de 95); brute 0,0 % (0 de 250) | según brazo | **cumple** |
| **Q6** | — | — | — | **no medido**: no es criterio de cierre de la Etapa 8 |

**Q4 no se cierra.** La salida lo dice: «met on the scopes that can be computed; NOT closed: 3 of 4 classes are not measurable». Una lectura secundaria (sin señal = orden simple) también da 1,000 frente a 0,990, y no sustituye a la primaria.

**Q5 por clase** (informativo, ranked): `temporal` 5 sin juzgar de 243 posiciones, `provenance` 2 de 124, `combined` 1 de 246, `vector_neighbourhood` 0 de 95. Ninguna pasa del 10 % (la mayor es 2,1 %).

### 2.1 Medias (corte 10, sin juzgar excluidos)

| Brazo | Ámbito | Consultas | recall@10 | nDCG@10 |
|---|---|---|---|---|
| simple | 100 consultas | 100 | 0,925 | 0,928 |
| ranked | 100 consultas | 100 | 0,927 | 0,933 |
| simple / ranked / hybrid | 25 con vector | 25 | 1,000 | 1,000 |
| vector (solo señal) | 25 con vector | 25 | 1,000 | 0,708 |
| brute | 25 con vector | 25 | 1,000 | 0,708 |
| simple | `temporal` / `provenance` / `combined` | 25 c/u | 0,932 / 0,892 / 0,876 | 0,962 / 0,852 / 0,896 |
| ranked | `temporal` / `provenance` / `combined` | 25 c/u | 0,912 / 0,885 / 0,911 | 0,932 / 0,839 / 0,960 |

Las cifras de las tres clases sin vector son de los brazos sin señal; no son de Q2 a Q4. El nDCG de 0,708 de `vector` y `brute` es el mismo número porque son la misma lista (Q1). No se ha analizado por qué es más bajo que el de `hybrid` (1,000); no era objeto de esta medición.

## 3. Rendimiento (§6.3)

### 3.1 P1 y P2: cocientes de p50 a 20 000 unidades

`derived_bench`: 20 000 unidades, 50 962 relaciones, 3 000 conjuntos de cambios, `KernoDb` sobre el sistema de ficheros real. Mismas repeticiones que la Etapa 7: tres invocaciones en procesos separados, cuatro corridas cada una (`--runs 4 --reps 12`, 512 pares por corrida y brazo, cuatro rondas de calentamiento), doce corridas publicadas. Los dos lados de cada cociente se miden en la misma sesión, par a par y alternando el orden. Sin umbral absoluto de latencia (D9).

| Umbral | Cota | Doce corridas (cociente de p50) | Estado |
|---|---|---|---|
| **P1** escritura con descriptor / sin descriptor | ≤ 1,50 en cada corrida | 1,165 a 1,238 (máximo 1,238, invocación 1 corrida 4) | **cumple**: 0 de 12 sobre el techo |
| **P2** búsqueda persistida / en memoria | ≤ 1,10 en cada corrida | 0,921 a 1,007 (máximo 1,007, invocación 1 corrida 4) | **cumple**: 0 de 12 sobre el techo |

Las tres invocaciones dan el veredicto «met» en su cierre; ninguna corrida se promedió ni se descartó. La carga media de 1 minuto antes de cada corrida estuvo entre 1,30 y 1,91 (máquina de 4 CPU), registrada en la salida.

Qué muestra: el descriptor y su vector cuestan entre un 17 % y un 24 % más de tiempo de mediana por escritura (p50 entre 0,15 y 0,23 ms), bien por debajo del techo; y leer los vectores persistidos de la base reabierta no cuesta más que tenerlos en memoria. Qué no muestra: P2 compara dos búsquedas sobre el mismo `VectorSet` ya cargado (la lectura única de 20 000 vectores servidos tarda unos 10 a 11 ms y se informa aparte, no entra en el cociente); no dice nada del coste en frío de abrir la base (reabrir: 156 a 162 ms). En la invocación 3 los cocientes de P2 quedan en 0,92 porque el lado en memoria resultó más lento (≈ 1,95 ms frente a ≈ 1,80 ms), no porque el persistido sea más rápido: el cociente es de dos medianas ruidosas y su diferencia de ±8 % es de ruido de la máquina. Los p99 se imprimen y no tienen umbral.

### 3.2 P3: invalidación, igualdad

En los dos motores: tras 286 cambios de fuente en 29 conjuntos de cambios, hay 2 400 derivados (2 000 vectores persistidos y 400 datos en memoria de dos fuentes), 400 esperados obsoletos y 400 obsoletos, y el resto `ready`. Tras reabrir: 286 datos persistidos obsoletos, 286 esperados, 1 714 servidos como `ready`, 0 degradados por la apertura. **Cumple**: el conjunto de obsoletos es exactamente el de los derivados de las fuentes que cambiaron, igualdad de conjuntos y no de contadores.

### 3.3 P4: reconstrucción, igualdad

En los dos motores: los 286 vectores reconstruidos tienen identidad, descriptor, vector y entrada **iguales byte a byte** a los de una construcción nueva sobre la misma historia (286 de 286), y no queda ningún obsoleto tras reconstruir. **Cumple.** El tiempo se informa sin umbral y no es comparable entre ejecuciones:

| Motor | Reconstruir 286 vectores | Construir de cero los mismos 286 |
|---|---|---|
| `emulated_log` | 1 493,7 ms | 1 170,1 ms |
| `kerno_db` | 19,2 ms | 11,8 ms |

La diferencia entre motores es la del simulador de disco y no la de los derivados. Reconstruir resultó más lento que construir de cero en ambos motores (≈ 1,3 y ≈ 1,6 veces). No se ha analizado la causa y no hay umbral sobre esto.

## 4. Tabla de umbrales

| Código | Estado | Nota |
|---|---|---|
| Q1 | cumple | 25 consultas con vector |
| Q2 | cumple | calculado sobre 25 de 100 consultas |
| Q3 | cumple | calculado sobre 25 de 100 consultas |
| Q4 | cumple donde se calcula; **no cerrado** | 3 de 4 clases no medibles (sin vector de consulta) |
| Q5 | cumple | máximo por brazo 1,1 % |
| Q6 | no medido | fuera del cierre, por el acta |
| P1 | cumple | 12 de 12 corridas, máximo 1,238 |
| P2 | cumple | 12 de 12 corridas, máximo 1,007 |
| P3 | cumple | igualdad, ambos motores |
| P4 | cumple | igualdad byte a byte, ambos motores |

## 5. Hallazgos

Ninguno sobre el código medido: no se tocó. La medición no encontró defectos.

## 6. Lo que no se afirma, y por qué

1. **Los vectores son deterministas y con semilla (D5).** Miden la mecánica de la búsqueda, de la persistencia y de la invalidación, no la calidad semántica de ningún modelo de embeddings. Que Q2 y Q3 den 1,000 dice que el recorrido con la señal ordena como el juicio sobre estos vectores, que el generador construyó a partir del mismo relato que los juicios: es una medición con poco margen de fallo, y no se debe leer como calidad de recuperación real.
2. **Q2, Q3 y Q4 se miden sobre 25 consultas**, no sobre 100. El acta los escribe «en conjunto» y «en cada clase»; el conjunto congelado no tiene vector de consulta en las otras tres clases. No se inventaron vectores de consulta para ellas (cambiaría el conjunto y exigiría congelarlo de nuevo). Por eso Q4 queda sin cerrar. Cerrarlo exige un conjunto con vectores en todas las clases, que es una decisión del propietario.
3. **Corpus, consultas y juicios los escribió el mismo equipo.** No hay juicio independiente (acta §6.1, límite declarado). Con 25 consultas por clase, las diferencias pequeñas no son significativas.
4. **Una sola ejecución de calidad** (determinista salvo los tiempos de P4). El brazo `ranked` usa los pesos por defecto, sin ajuste; Q6 (ranking frente a orden simple) no se midió y no se debe deducir de estas cifras.
5. **Carga de la máquina durante las corridas**: media de 1 minuto entre 1,30 y 1,91 en 4 CPU, sin otra carga lanzada a propósito; está en la salida. Los cocientes de P1 y P2 son de dos medianas de la misma sesión, así que la carga común se cancela en parte, pero no se afirma ningún tiempo absoluto.
6. **P1 compara dos escrituras de forma distinta**: con descriptor se crea la unidad y se persiste su vector en un conjunto de cambios; sin descriptor solo se crea la unidad. Mide el sobrecoste de añadir el derivado a la escritura, no el de calcular el vector (que lo entrega quien llama, ADR-0011).
7. **No medido**: 100 000 unidades, concurrencia entre lecturas y escrituras, vectores de otra dimensión, consolidación (resúmenes) por rendimiento, búsqueda aproximada.
8. **No hecho en este paquete**: la mutación de cada prueba nueva sobre una copia del árbol (acta §4 punto 4). Las pruebas `a_manifest_that_does_not_match_refuses_to_measure` y `unjudged_units_are_excluded_and_counted` comprueban los dos puntos delicados de la medición, pero no se vio fallar cada una con una mutación registrada.

## 7. Reproducción

```text
# calidad, P3 y P4 (cada vez que se ejecuta cargo test)
cargo test -p kerno-units --release --test derived_measure -- --nocapture --exact quality_over_the_frozen_set

# P1 y P2 (no se ejecuta con cargo test; tres invocaciones de cuatro corridas)
cargo run --release -p kerno-units --example derived_bench -- --runs 4 --reps 12 --dir <directorio temporal>
```

La salida cruda [etapa8-medicion-salida.txt](etapa8-medicion-salida.txt) es la de estas órdenes, en este orden: la prueba de calidad y luego las tres invocaciones.

## 8. Verificación

| Comprobación | Orden | Resultado |
|---|---|---|
| Formato | `cargo fmt --all --check` | 0 |
| Lints | `cargo clippy --workspace --all-targets --all-features -- -D warnings` | 0 |
| Suite | `cargo test -p kerno-testkit -p kerno-units --all-features --release` | 0; **856 aprobadas, 0 fallidas**. `derived_measure.rs` aporta 4 pruebas |
| `Cargo.toml` / `Cargo.lock` | — | sin cambios |
