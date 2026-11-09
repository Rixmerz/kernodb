# Experimento de hilos narrativos: resultado

**Veredicto: NO-GO.** Los hilos narrativos no superan a la búsqueda vectorial simple. En cobertura de evidencia quedan un 14 % por debajo (0,437 frente a 0,507), cuando el umbral exigía al menos un 10 % por encima. Además, la relación justificable está en 72 % y el umbral era ≥ 85 %. La recuperación narrativa no se integra en Kerno ni toca el formato persistente; la configuración usada queda congelada y trazable aquí.

## Desviación del protocolo

El propietario delegó las revisiones el 2026-11-09 («No voy a revisarlo, hazlo tú»). La revisión de preguntas y de relaciones no la hizo una persona independiente, sino `claude-opus-5-5`, un modelo que no escribió las preguntas (`claude-haiku-5-5`) ni construyó los hilos (`claude-sonnet-5-5`). Esa revisión decidió mantener, editar o descartar cada pregunta antes de congelarlas, y ninguna estrategia vio una pregunta antes de la congelación (`evaluate.py` comprueba los SHA-256 de `questions/frozen/MANIFEST.json`).

## Configuración

| Elemento | Valor |
|---|---|
| Corpus | 6 documentos, `corpus/MANIFEST.json` (hashes, licencias) |
| Preguntas | 180 borradores q3 (Haiku), 172 congeladas tras la revisión (8 descartadas, 100 editadas) |
| Extracción | x2, Sonnet; Moby Dick en 5 ventanas de ≤ 300 kB (ningún hilo cruza ventanas) |
| Embeddings | `BAAI/bge-small-en-v1.5` local (fastembed) |
| Recuperación | presupuesto de 6000 bytes, 3 semillas, radio 1 (`retrieve.CONFIG`) |
| Llamadas a modelos | aisladas: sin ajustes, plugins, hooks, herramientas ni MCP |

## Resultados de recuperación (172 preguntas)

| Estrategia | Cobertura de evidencia | Precisión de bloques |
|---|---|---|
| vectorial | **0,507** | **0,061** |
| vectorial con contexto | 0,485 | 0,050 |
| hilos narrativos | 0,437 | 0,037 |

Por documento (cobertura): los hilos ganan solo en Moby Dick (0,26 frente a 0,24, n = 28) y pierden en los otros cinco; en *A Study in Scarlet* caen a 0,23 frente a 0,43. Por tipo, también pierden en las preguntas de trayectoria, para las que se diseñaron (0,40 frente a 0,48, n = 81). Pregunta a pregunta: mejores en 27, peores en 51, empate en 94.

## Umbrales GO

| Criterio | Umbral | Medido | |
|---|---|---|---|
| Cobertura frente a la mejor línea base | ≥ +10 % | −14 % (0,437 / 0,507) | no cumple |
| Caída de precisión | ≤ 5 pp | −2,4 pp | cumple |
| Relaciones justificables | ≥ 85 % | 39 de 54 = 72 % (muestra 1 de cada 4) | no cumple |
| Coherencia de hilos (≥ 80 %) | evaluación ciega | no medida | — |
| Respuestas sin evidencia inventada (≥ 95 %) | evaluación ciega | no medida | — |
| Reproducibilidad | misma versión y configuración | sí: versiones, hashes, ledger por llamada | cumple |

Las respuestas con Haiku y la evaluación ciega no se ejecutaron: con dos criterios obligatorios ya incumplidos el veredicto no puede cambiar, y la partida de evaluación (USD 2,94 de 3) no alcanza para juzgar 516 respuestas. `answer.py` y `eval_sheets.py --answers` quedan listos si se quiere medir igualmente.

## Lectura

- Los hilos sustituyen bloques similares a la pregunta por bloques de segmentos relacionados; con un presupuesto fijo eso desplaza evidencia que la búsqueda vectorial ya encontraba. El extractor produce segmentos gruesos (en Moby Dick, unos 8 kB de media) y pocas relaciones, y una de cada cuatro relaciones muestreadas no se sostiene con sus citas.
- La única señal positiva está en el texto más largo, que es justo donde la búsqueda vectorial cubre peor (0,24). Una hipótesis para otra ronda —con preguntas nuevas, porque estas ya no sirven para ajustar— sería usar los hilos solo como reordenación de candidatos vectoriales en documentos largos, no como sustitución.

## Coste

USD 6,49 de 25: extracción 3,55 de 15 (incluye la versión x1 descartada), evaluación y preguntas 2,94 de 3 (incluye las pasadas q1 y q2 descartadas por no estar aisladas), respuestas 0 de 7. Cada llamada está en `results/ledger.jsonl` con su sesión, sus modelos y ambas cifras de coste.
