# Kerno

**Versioned Embedded Data Engine** — motor de datos embebido y local-first que combina persistencia transaccional, evolución versionada de los datos e integridad verificable, con una ruta de extensión hacia la recuperación híbrida para aplicaciones de IA.

> Un dato se almacena una vez, sus representaciones derivadas se mantienen coherentes, su evolución puede reconstruirse y sus políticas de acceso y conservación son explícitas.

## Estado

**Fase 0 / M0 completada, pendiente de aprobación.** Todavía no existe el motor de Kerno ni una API utilizable: este repositorio contiene la arquitectura de referencia, los ADR y un *spike* que compara dos arquitecturas de almacenamiento (copy-on-write B+ tree y LSM tree) con las mismas pruebas de fallos y los mismos benchmarks.

**Recomendación de M0 ([ADR-0001](docs/adr/0001-motor-de-almacenamiento.md)): LSM nivelado.** Ambos candidatos superan las mismas pruebas de caídas, fallos de E/S y corrupción; en los casos de referencia de Kerno el LSM rinde entre 1,6× y 8× más en escritura (2,8× en commits durables tipo Cauce, 6,9× escribiendo historial), escribe ~100× menos bytes por commit pequeño y mantiene un prefijo consistente sin `fsync` por commit. El B+ tree conserva mejores colas de latencia y lecturas de historial más rápidas. Detalle en el [informe de M0](docs/testing/m0-report.md).

No crees bases de datos reales con el código de `spikes/`: los formatos de los prototipos no están congelados y se descartarán o reescribirán en la Fase 1.

## Documentación

| Documento | Contenido |
|---|---|
| [SAD v0.1.0](docs/architecture/SAD.md) | Arquitectura de referencia y plan de implementación (baseline) |
| [ADR](docs/adr/README.md) | Decisiones de arquitectura y su estado |
| [ADR-0001](docs/adr/0001-motor-de-almacenamiento.md) | Motor de almacenamiento: recomendación con la evidencia de M0 |
| [Invariantes y riesgos](docs/architecture/invariants.md) | Lo que ninguna funcionalidad puede romper, y cómo se prueba |
| [Matriz de fallos](docs/architecture/failure-matrix.md) | Qué ocurre si el proceso o la energía caen en cada punto del commit |
| [Informe de M0](docs/testing/m0-report.md) | Resultados de pruebas y benchmarks |
| [Plan de pruebas](docs/testing/test-plan.md) | Cómo se verifican las garantías |
| [Roadmap](docs/roadmap/README.md) | Estado por fase y checklist de M0 |

## Estructura

```text
docs/                    arquitectura, ADR, formato, seguridad, pruebas, roadmap
spikes/storage/          spike de M0 (no es el motor de Kerno)
  spike-core/            contrato común, VFS con simulación de fallos, suites de pruebas compartidas
  spike-btree/           prototipo copy-on-write B+ tree
  spike-lsm/             prototipo LSM tree
  spike-bench/           benchmarks M0 (+ redb, fjall y SQLite como referencia)
  run-benchmarks.sh      reproduce los benchmarks con un único comando
```

La estructura de crates del producto (SAD §14) se creará en la Fase 1, cuando los contratos estén estables.

## Desarrollo

Requisitos: Rust ≥ 1.89 (probado con 1.97), Linux. Los benchmarks de referencia compilan SQLite (necesitan un compilador de C).

```bash
cargo test --workspace --exclude spike-bench          # todas las suites de los prototipos
cargo test -p spike-bench --features candidates --release   # prueba diferencial entre motores
spikes/storage/run-benchmarks.sh                       # benchmarks de M0 (ver ADR-0012)
```

Convenciones de contribución en [CONTRIBUTING.md](CONTRIBUTING.md).

## Licencia

Pendiente de decisión (SAD §20, tarea 1). Hasta que se publique un archivo `LICENSE`, no se concede ninguna licencia de uso del código de este repositorio.
