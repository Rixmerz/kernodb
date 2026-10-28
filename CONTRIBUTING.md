# Cómo contribuir a Kerno

## Idioma

- Documentación de arquitectura, ADR y planes: **español**.
- Código, comentarios, mensajes de commit, nombres y documentación de API (`///`): **inglés**, como es habitual en el ecosistema Rust.

## Antes de escribir código

1. Comprueba si el cambio toca una decisión abierta de [SAD §2.2](docs/architecture/SAD.md#22-decisiones-que-todavía-no-deben-asumirse-como-resueltas). Si es así, el ADR correspondiente debe estar aceptado antes de codificar el supuesto de forma difícil de revertir.
2. Identifica las [invariantes](docs/architecture/invariants.md) afectadas y la prueba que las verifica.
3. Los cambios de formato persistente, protocolo de commit o recuperación requieren actualizar ADR-0002/ADR-0003 y la [matriz de fallos](docs/architecture/failure-matrix.md).

## Definition of Done (SAD §18.2)

Un cambio está terminado cuando:

- el contrato de API y errores está documentado;
- las invariantes afectadas están definidas y probadas;
- pasan las pruebas normales **y de fallo** relevantes;
- se evaluaron efectos sobre formato, migración, historial, índices y backups;
- se evaluaron consecuencias de seguridad y privacidad;
- hay métricas de rendimiento cuando corresponde;
- los ADR relacionados están actualizados;
- la documentación describe el comportamiento real, sin garantías no probadas.

## Comprobaciones locales

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --exclude spike-bench
```

Las suites aleatorias aceptan `SPIKE_SEEDS=<n>` (más semillas) y `SPIKE_SEED=<n>` / `SPIKE_POINT=<n>` (reproducir un fallo). Ver el [plan de pruebas](docs/testing/test-plan.md).

## Reglas para el código del motor

- Todo acceso a archivos pasa por la abstracción `Vfs`; nunca `std::fs` directo en código del motor (es lo que permite probar caídas y corrupción).
- Ningún dato leído del disco se usa sin validar longitudes, límites y checksum.
- Tras un `fsync` fallido no se reintenta asumiendo que los datos llegaron: el handle queda envenenado.
- Sin criptografía propia: solo algoritmos estándar de bibliotecas consolidadas (SAD §3.7).
- El núcleo no puede depender de red, de un proveedor de IA ni de un modelo (I-31).

## Dependencias y licencias

- Kerno se licencia como `MIT OR Apache-2.0` ([LICENSE-MIT](LICENSE-MIT), [LICENSE-APACHE](LICENSE-APACHE)); toda contribución se acepta bajo esas mismas dos licencias. Las dependencias nuevas del núcleo deben tener una licencia compatible (MIT, Apache-2.0, BSD, ISC, Zlib o equivalente permisiva).
- No se copia código de otros proyectos sin revisar su licencia y registrarlo. Leer el diseño de proyectos de referencia (redb, RocksDB, LevelDB, LMDB, SQLite) está bien; copiar su implementación no.
- Cada dependencia nueva del núcleo debe justificarse en el PR (tamaño, mantenimiento, licencia). Las dependencias usadas solo en benchmarks viven en `spike-bench` o `benchmarks/`.
- `Cargo.lock` se versiona.

## Commits y PR

- Commits pequeños con mensaje en inglés en imperativo (`Add …`, `Fix …`) y cuerpo que explique el porqué.
- Un PR que cambie rendimiento adjunta la comparación con el baseline (ADR-0012).
