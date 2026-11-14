# coqui-transport-guatiao

A **transport written in any language Guatiao hosts, chosen by provider id like any other extension.** The `transport` kind dials
and listens, and each connection is a Guatiao channel of whole messages. A transport of `coqui-transport` becomes a provider with
one macro, in the crate that holds its types, so neither `coqui-transport` nor the transport's own crate depends on Guatiao.

!!! note "Alpha"
    `0.0.0-alpha.0` is unreleased: depend on a git revision. The kind may change in any commit, and a change moves a floor
    hash, so every provider is rebuilt.

## Installation

```bash
cargo add --git https://github.com/Boriken-Dev/coqui-transport-guatiao coqui-transport-guatiao --features pipe
```

| Feature | Adds | Needed for |
| --- | --- | --- |
| `pipe` | the `pipe` provider, over `coqui-transport-pipe` | named pipes and Unix domain sockets |
| `websocket` | the `websocket` provider, over `coqui-transport-web` | plain WebSocket |
| `contract` | the suite every provider passes | writing a provider |

## 30-second tour

A host registers the built-in transport, picks it by id and dials:

```rust
let mut registry = Registry::new("my-host", "1.0");
pipe::register(&mut registry)?;
let pipe = registry
    .offer::<dyn Transport>("pipe")
    .ok_or("no provider has that id")?
    .map_err(|_| "the provider is not a transport")?
    .instantiate(&Value::from(Map::new()))?;
let channel = pipe.dial(DialRequest::new("pipe:my-service"), sink)?;
channel.send(b"hello")?;
```

A transport written in Python is a module with a class that inherits `guatiao.provide.Provider` and the kind; the host adds
Guatiao's source of Python libraries to the registry and picks it by id the same way. A native library calls
`guatiao::providers!` on a provider the macro wrote.

## Learn more

- [Rust API](https://boriken-dev.github.io/coqui-transport-guatiao/rust/index.html)
- [Changelog](https://github.com/Boriken-Dev/coqui-transport-guatiao/blob/main/CHANGELOG.md)
