# ADR-0011 — Perfil de vector y recuperación híbrida

| Campo | Valor |
|---|---|
| Estado | Aceptado (2026-11-11, por delegación del propietario; revocable) |
| Fecha | 2026-10-27 (apertura); 2026-11-11 (aceptado en E7-00) |
| Decisores | Propietario del proyecto (delegación de 2026-11-11, [acta de la Etapa 7](../architecture/etapa7-acta.md) §7.1 y §8) |
| Relacionado con | SAD §10, D-08, invariante I-25; [acta de la Etapa 7](../architecture/etapa7-acta.md) §2.3, §4, §7; [conjunto de evaluación](../architecture/etapa7-conjunto-evaluacion.md); ADR-0007, ADR-0004, ADR-0018, ADR-0009; invariantes I-25, I-31 |

## Contexto

La recuperación híbrida combina texto, vectores y filtros. Los vectores son derivados de una revisión fuente y de un perfil de modelo.

**Pregunta:** Qué contratos comunes definen embeddings, métricas, filtros y frescura de índices.

La Etapa 7 necesita una señal vectorial opcional para el ranking (E7-04) sin fijar nada que sea difícil de revertir. Un vector almacenado sería un derivado (I-25): tendría que atarse a su revisión fuente y a su perfil de modelo, y su frescura depende de ADR-0018, que se decide antes de la Etapa 8. Por eso esta etapa no persiste vectores.

## Opciones consideradas

1. **Búsqueda exacta (fuerza bruta) primero** como baseline de recall.
2. **ANN (HNSW u otro) desde el inicio**.
3. **Delegar en una biblioteca externa** de búsqueda vectorial.

## Propuesta inicial (SAD §15)

Vectores exactos primero; metadatos de modelo y revisión fuente; generación mediante adaptadores; ANN solo con evidencia.

## Criterios de decisión

Comparación con baseline exacto, verificación de dimensiones/métrica y prueba de invalidación tras actualizar/borrar el documento.

Criterios fijados en la [acta de la Etapa 7](../architecture/etapa7-acta.md) §7 antes de medir: (a) una opción no puede fijar supuestos de persistencia que la Etapa 7 no necesita; (b) el núcleo no depende de red ni de un modelo (I-31); (c) ANN solo con un benchmark que lo justifique.

## Lo que aportó M0

El workload `agent_memory` almacena embeddings de 768 × f32 (3 KiB) como valores normales y mide escritura, lectura y espacio en ambos candidatos. Valores de este tamaño son relevantes para ADR-0001 (overflow pages en B+ tree, bloques grandes en LSM).

## Evidencia

- Aceptación en E7-00 con el conjunto de evaluación congelado y sus líneas base B0 y B1 ([conjunto de evaluación](../architecture/etapa7-conjunto-evaluacion.md), §4). La búsqueda exacta todavía no existe: su medición (recall frente a §6.2 del acta) llega en E7-04 y E7-05.
- Ningún cambio de formato persistente en la Etapa 7. El código de E7-00 no calcula similitudes ni almacena vectores.

## Decisión

**Opción 1: búsqueda exacta primero**, con este límite para la Etapa 7:

- **Los vectores los entrega quien llama y viven solo en memoria.** El núcleo no genera embeddings ni llama a modelos (I-31). Nada se escribe en el formato de unidades ni en los índices.
- **Contrato de dimensión.** Un conjunto de vectores tiene una dimensión fija, mayor que cero. Un vector de otra dimensión, o una consulta de otra dimensión, se rechaza con un error; no hay relleno ni recorte.
- **Contrato de métrica.** Dos métricas: coseno y producto escalar. Una consulta con métrica distinta de la del conjunto se rechaza. Un vector con componentes no finitos se rechaza. Con coseno, un vector de norma cero se rechaza, porque su coseno no está definido.
- **Persistir vectores como derivados ligados a su revisión fuente (I-25) queda para la Etapa 8**, con ADR-0018 (versionado de derivados) y ADR-0009 (supresión). Ningún cambio del formato persistente en esta etapa.
- **Sin ANN sin benchmark.** Un índice aproximado solo entra con una medición que lo justifique y con un ADR que lo acepte.

## Consecuencias

- **Más fácil:** la búsqueda exacta es correcta por construcción y sirve de referencia de recall para cualquier índice futuro. Los contratos de dimensión y métrica son pequeños y se prueban en E7-04.
- **Más difícil:** los vectores se recalculan en cada consulta o los entrega quien llama en cada uso. Para conjuntos grandes el coste lineal puede ser alto; la medición de §6.3 del acta (20 000 unidades) lo registra sin umbral.
- **Comprometido:** la API de consulta vectorial recibe los vectores como argumento y no los lee del almacén. Cambiar eso (vectores persistidos, ANN) es un cambio de decisión y requiere un ADR que reemplace a este; un ADR aceptado no se edita para cambiarlo.
- **Revisar:** cuando se decida ANN o la persistencia de vectores (Etapa 8). Si la medición de E7-05 muestra que la búsqueda exacta no cumple un límite de latencia acordado, eso abre una decisión nueva, no una edición de esta.

## Seguimiento

- E7-04: implementar los contratos de dimensión y métrica y la búsqueda exacta; pruebas de rechazo por dimensión y por métrica.
- E7-05: medir la señal vectorial exacta sobre el conjunto congelado frente a §6.2 y registrar el coste de §6.3.
- Etapa 8: decidir la persistencia de vectores con ADR-0018 y ADR-0009 antes de escribir ningún derivado.

## Lo que se mantuvo del ADR de apertura

Las opciones 2 y 3 quedan descartadas para la Etapa 7: ANN necesita un benchmark que aún no existe (§2.3 del acta), y una biblioteca externa de búsqueda vectorial añadiría una dependencia que el núcleo no necesita para la búsqueda exacta.
