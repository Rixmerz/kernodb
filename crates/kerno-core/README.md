# kerno-core

The shared contract of [Kerno](https://github.com/Rixmerz/kernodb): the error model, write batches, durability
modes and the `Vfs` file abstraction that the storage engine uses.

> **Alpha (1.0.0-alpha.1).** Part of the Kerno alpha. Applications should depend on the `kerno` crate, which
> re-exports what they need; this crate is published because `kerno` depends on it.

```sh
cargo add kerno-core@1.0.0-alpha.1
```

Licensed under either of the Apache License, Version 2.0 or the MIT license, at your option.
