# ADR-0011 — Perfil de vector y recuperación híbrida

| Campo | Valor |
|---|---|
| Estado | Pendiente |
| Fecha | 2026-10-27 (apertura) |
| Relacionado con | SAD §10, D-08, invariante I-25 |

## Contexto

La recuperación híbrida combina texto, vectores y filtros. Los vectores son derivados de una revisión fuente y de un perfil de modelo.

**Pregunta:** Qué contratos comunes definen embeddings, métricas, filtros y frescura de índices.

## Opciones consideradas

1. **Búsqueda exacta (fuerza bruta) primero** como baseline de recall.
2. **ANN (HNSW u otro) desde el inicio**.
3. **Delegar en una biblioteca externa** de búsqueda vectorial.

## Propuesta inicial (SAD §15)

Vectores exactos primero; metadatos de modelo y revisión fuente; generación mediante adaptadores; ANN solo con evidencia.

## Criterios de decisión

Comparación con baseline exacto, verificación de dimensiones/métrica y prueba de invalidación tras actualizar/borrar el documento.

## Lo que aportó M0

El workload `agent_memory` almacena embeddings de 768 × f32 (3 KiB) como valores normales y mide escritura, lectura y espacio en ambos candidatos. Valores de este tamaño son relevantes para ADR-0001 (overflow pages en B+ tree, bloques grandes en LSM).

## Dependencias

Depende de ADR-0007 (frescura) y ADR-0004 (revisión fuente).

## Decisión

Pendiente. No debe codificarse ningún supuesto de este ADR de forma difícil de revertir hasta que se acepte.
