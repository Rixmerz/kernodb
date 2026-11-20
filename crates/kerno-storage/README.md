# kerno-storage

The storage engine of [Kerno](https://github.com/Rixmerz/kernodb): a leveled LSM tree with a write-ahead log,
a manifest and transactions. All file access goes through the `Vfs` of `kernodb-core`. Its design, formats,
fsync counts and recovery are described in `DESIGN.md` in this package.

> **Alpha (1.0.0-alpha.1).** The on-disk format v1 is frozen
> ([ADR-0013](https://github.com/Rixmerz/kernodb/blob/main/docs/adr/0013-format-v1-and-engine-protocol.md)).
> Applications should use the `kerno` crate; this crate is published because `kerno` depends on it.

```sh
cargo add kernodb-storage@1.0.0-alpha.1
```

Licensed under either of the Apache License, Version 2.0 or the MIT license, at your option.
