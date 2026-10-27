# Spike de almacenamiento (M0)

Prototipos comparables de las dos arquitecturas candidatas de [ADR-0001](../../docs/adr/0001-motor-de-almacenamiento.md), sometidos a las mismas pruebas y benchmarks. **No es el motor de Kerno**: los formatos no están congelados y el código se reescribirá o se adaptará en la Fase 1.

| Crate | Contenido |
|---|---|
| [`spike-core`](spike-core) | Contrato `Engine`/`Snapshot`, abstracción `Vfs` (`StdVfs`, `SimVfs` con simulación de caídas, `CountingVfs`), suites compartidas (`engine_test_suite!`) y `LogEngine` de referencia |
| [`spike-btree`](spike-btree) | Copy-on-write B+ tree con meta doble, checksums Merkle y reutilización de páginas por snapshot — [DESIGN.md](spike-btree/DESIGN.md) |
| [`spike-lsm`](spike-lsm) | LSM nivelado con WAL, memtable en skiplist, SST con bloom filter y compactación en segundo plano — [DESIGN.md](spike-lsm/DESIGN.md) |
| [`spike-bench`](spike-bench) | Driver de benchmarks, workloads de SAD §17.3 y de los casos de referencia, adaptadores de redb, fjall y SQLite |

## Pruebas

```bash
cargo test -p spike-core
cargo test -p spike-btree
cargo test -p spike-lsm
cargo test -p spike-bench --features candidates --release   # diferencial entre motores
SPIKE_SEEDS=400 cargo test -p spike-btree -p spike-lsm --release
```

Detalle en el [plan de pruebas](../../docs/testing/test-plan.md).

## Benchmarks

```bash
spikes/storage/run-benchmarks.sh            # small + medium, 3 repeticiones
SCALES=small,medium,large REPS=5 spikes/storage/run-benchmarks.sh
```

Metodología en [ADR-0012](../../docs/adr/0012-metodologia-de-benchmarks.md); resultados en [`results/`](results) y en el [informe de M0](../../docs/testing/m0-report.md).
