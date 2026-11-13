# ADR-0019 — Licencia y contribuciones

| Campo | Valor |
|---|---|
| Estado | **Aceptado** por el propietario del proyecto el 2026-11-11 |
| Fecha | 2026-11-11 |
| Decisores | Propietario del proyecto |
| Relacionado con | ADR-0010 (API pública y límites de crates); [`Cargo.toml`](../../Cargo.toml), [`LICENSE-MIT`](../../LICENSE-MIT), [`LICENSE-APACHE`](../../LICENSE-APACHE), [`CONTRIBUTING.md`](../../CONTRIBUTING.md) y la sección «Licencia» de [`README.md`](../../README.md) |

## Contexto

El motor es una biblioteca embebida. Hoy la licencia está declarada en dos sitios que coinciden: `license = "MIT OR Apache-2.0"` en `[workspace.package]` de `Cargo.toml`, que heredan `kerno-core` y `kerno-storage` con `license.workspace = true`, y los textos `LICENSE-MIT` y `LICENSE-APACHE`. La sección «Licencia» del README añade que una contribución enviada para su inclusión se licencia con esas dos licencias, «sin términos ni condiciones adicionales». No existe un contrato de contribución: ni el README ni `CONTRIBUTING.md` lo mencionan.

Hay que decidir ahora porque las condiciones de cada contribución externa quedan fijadas con su primer envío. Sin un acuerdo previo, el propietario no podrá volver a licenciar el código ajeno ni ofrecer términos comerciales sin pedir permiso a cada autor. También hay que fijar de dónde puede venir el ingreso, si lo hay, y cuándo se empieza a considerar cobrar.

Esta decisión no cambia el formato ni la API: el formato v1 está congelado desde el 2026-10-31 ([ADR-0013](0013-formato-v1-y-protocolo-del-motor.md)) y los límites de crates están aceptados ([ADR-0010](0010-api-publica-y-crates.md)).

## Opciones consideradas

1. **Permisiva, «MIT OR Apache-2.0» (elegida).** Cualquiera puede usar, modificar e integrar el núcleo en productos cerrados sin publicar su código. *Adopción:* la más alta y la de menos fricción legal. *Quién paga:* nadie por el motor; el ingreso, si lo hay, viene de lo que queda fuera del núcleo. *Coste:* quien ofrezca el motor como servicio no tiene que pagar por él.
2. **AGPL-3.0 con licencia comercial.** El núcleo obliga a publicar los cambios de quien lo ofrezca como servicio en red. *Adopción:* menor en organizaciones con política contra AGPL, que son también las que compran. *Quién paga:* quien no puede cumplir AGPL compra la licencia comercial. Exige CLA, porque la licencia comercial requiere poder licenciar el código de todos los autores.
3. **Licencia de código visible con conversión diferida.** El código se publica, pero el uso comercial se restringe durante un plazo y después pasa a una licencia abierta. *Adopción:* menor, porque no es una licencia de código abierto aprobada y muchas organizaciones la tienen vetada. *Quién paga:* quien usa el motor durante el plazo restringido. Exige CLA.
4. **Núcleo abierto y funciones de empresa de pago.** El núcleo es permisivo; las funciones de empresa se distribuyen con licencia propietaria. *Adopción:* alta en el núcleo, pero la frontera entre lo abierto y lo de pago genera fricción con quien contribuye. *Quién paga:* quien necesita las funciones de empresa. Exige separar el código y CLA para mantener ambas partes.

## Criterios de decisión

Los criterios que el propietario fijó al decidir:

- adopción sin fricción legal para quien usa el núcleo;
- que el propietario conserve la opción de volver a licenciar y de ofrecer términos comerciales;
- que el ingreso, si lo hay, venga de algo distinto del motor;
- que no se cobre por el motor antes de un 1.0 con formato estable, migraciones y usuarios reales en producción.

## Evidencia

- `Cargo.toml` declara `license = "MIT OR Apache-2.0"` en `[workspace.package]`; `crates/kerno-core/Cargo.toml` y `crates/kerno-storage/Cargo.toml` heredan la licencia con `license.workspace = true`. Comando: `grep -n license Cargo.toml crates/kerno-core/Cargo.toml crates/kerno-storage/Cargo.toml`.
- La sección «Licencia» del README licencia la contribución con las dos licencias, sin términos adicionales. No menciona ningún CLA; tampoco lo hace `CONTRIBUTING.md`.
- El formato v1 está congelado y ADR-0013 aceptado el 2026-10-31 ([`v1.md`](../file-format/v1.md)). ADR-0010 está aceptado el 2026-10-31.
- Las migraciones con fixtures de versiones previas están fuera del adelanto de la Etapa 9 y siguen pendientes ([acta de adelanto, §2.3](../architecture/etapa9-adelanto-acta.md)).

## Decisión

El núcleo se distribuye bajo «MIT OR Apache-2.0», tal como declaran `Cargo.toml` y los textos `LICENSE-MIT` y `LICENSE-APACHE`. Antes de aceptar la primera contribución externa hace falta un contrato de contribución (CLA) que mantenga al propietario la opción de volver a licenciar o de ofrecer términos comerciales. Si hay ingresos, vendrán de lo que queda fuera del núcleo: sincronización como servicio, funciones de empresa y soporte. No se cobra por el motor, y cobrar por él no se considera antes de un 1.0 con formato estable, migraciones y usuarios reales en producción.

## Consecuencias

- **Más fácil:** que cualquiera integre el núcleo sin pedir permiso ni pagar; que el propietario pueda ofrecer términos comerciales a quien los pida, siempre que el CLA cubra el código.
- **Más difícil:** quien ofrezca Kerno como servicio no paga al propietario. El ingreso depende de servicios y funciones que todavía no existen.
- **Comprometido:** cada contribución externa necesita el CLA desde la primera; sin él, el propietario no puede volver a licenciar ese código. Hasta que exista el CLA, la única regla sobre contribuciones es la sección «Licencia» del README, que no lo menciona.
- **Revisar:** este ADR, antes de cualquier cobro y antes de un 1.0.

## Seguimiento

- **Texto del CLA:** no está escrito. Es un seguimiento de este ADR. Debe decidir si el contribuidor cede sus derechos o concede una licencia amplia, y debe existir antes de aceptar la primera contribución externa.
- **README y `CONTRIBUTING.md`:** cuando exista el CLA, la sección «Licencia» del README y `CONTRIBUTING.md` deben remitir a él. Esos archivos están fuera de `docs/` y no se modifican en esta tarea.
- **Cobro:** reabrir este ADR cuando se cumplan las condiciones de un 1.0 con formato estable, migraciones y usuarios reales en producción.
- **Borrador (2026-11-13):** existe la propuesta [cla-propuesta.md](../legal/cla-propuesta.md), en borrador y sin vigor. Espera la revisión de un abogado y la aprobación del propietario. El punto «Texto del CLA» de arriba sigue valiendo: el texto no está aprobado.
