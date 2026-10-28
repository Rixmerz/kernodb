# ADR-0014 — Representación física de relaciones

| Campo | Valor |
|---|---|
| Estado | Propuesto |
| Fecha | 2026-10-28 |
| Decisores | Propietario del proyecto (pendiente) |
| Relacionado con | [especificación v0.2.0](../spec/especificacion-arquitectonica-v0.2.0.md) §6.4, §10, §12; SAD §9 (tabla de estructuras, fila 7); ADR-0001, ADR-0004, ADR-0007, ADR-0010; invariantes I-21, I-39 |

## Contexto

La Etapa 4 introduce relaciones tipadas (`Edge`) entre unidades: `edge_id`, `from_unit_id`, `to_unit_id`, `relation_type`, peso y validez temporal opcionales, procedencia y atributos. La especificación pide no asumir que todas son dirigidas ni que tienen peso, y separar relaciones declaradas de inferidas. El SAD (§1 y §20, «Fuera de la primera tanda») excluye un lenguaje de grafos y reserva un motor de grafos especializado para cuando un workload lo justifique.

El motor físico es un LSM nivelado (ADR-0001) con lotes atómicos, aislamiento por instantánea y recorridos por rango. La pregunta es cómo codificar relaciones sobre ese motor.

## Opciones consideradas

1. **Relaciones como unidades** con un tipo reservado, sin índice de adyacencia. Mínima superficie; recorrer vecinos exige un filtro sobre todas las relaciones.
2. **Registro de relación + índices de adyacencia transaccionales** (propuesta). Un registro por `edge_id` y dos entradas de índice con claves ordenadas `out/{from}/{type}/{edge_id}` e `in/{to}/{type}/{edge_id}`, escritos en el mismo lote que el registro. Vecinos por rango; coste: tres escrituras por relación.
3. **Listas de adyacencia por unidad** (un valor con todos los vecinos). Lectura de vecinos en una sola clave; reescritura completa de la lista en cada cambio y conflictos de escritura en nodos con muchas relaciones.
4. **Motor de grafos dedicado**. Fuera de alcance hasta que un benchmark lo justifique (SAD, spec §5).

## Criterios de decisión

Fijados antes de medir:

1. Atomicidad: registro e índices de una relación se confirman en la misma frontera que el resto del cambio (I-21).
2. Equivalencia: recorrer vecinos por índice devuelve lo mismo que filtrar todas las relaciones (ruta de referencia, I-32).
3. Coste por relación (bytes escritos, escrituras) y latencia de recorrido de 1 y 2 saltos en los workloads de LiveSpec y memoria de agentes.
4. Borrar o archivar una unidad tiene una semántica explícita para sus relaciones (rechazo, cascada o relaciones colgantes marcadas).
5. Las relaciones inferidas se distinguen físicamente o por un campo obligatorio de las declaradas (I-39).

## Evidencia

Ninguna todavía. La Etapa 4 debe aportar un prototipo de la opción 2 frente a la 1 sobre `kerno-storage`, con los workloads anteriores.

## Decisión

Pendiente. Propuesta: opción 2, con dirección y peso opcionales por `relation_type`, procedencia obligatoria (`declared`, `imported`, `algorithmic`, `inferred`) y, para las inferidas, origen y confianza.

## Consecuencias

- Las claves de adyacencia pasan a formar parte del esquema lógico de claves; su codificación se versiona con el resto del formato lógico (ADR-0003, ADR-0008).
- Las relaciones con validez temporal interactúan con el historial (ADR-0004): hay que decidir si una relación tiene revisiones propias o solo intervalo de validez.
- Una relación hacia una unidad suprimida queda sujeta a ADR-0009.

## Condiciones para reabrir

Workloads con recorridos de más de dos saltos dominantes, o nodos con más de 10⁵ relaciones donde el coste de la opción 2 sea inaceptable.

## Seguimiento

- Prototipo y benchmark al inicio de la Etapa 4.
- Prueba de propiedades: índices de adyacencia coherentes con los registros tras fallos (matriz de fallos).
