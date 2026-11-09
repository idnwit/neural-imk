# 0004. All persistent data goes through the `Storage` trait

> **Status:** accepted · **Date:** 2026-11-08

## Context

The log records every key the user types. The PoC stores it as plaintext on purpose, but later versions will need encryption.
Replay and tests must run in memory without touching real files.

## Decision

- Event log, model snapshots, config and the replay buffer go through `Storage` (`EventSink`, `EventSource`, `BlobStore`) in `nimk-storage`.
- `Storage` only sees `&[u8]`; serialization lives in the layer above.
- The PoC ships `PlainFsStorage` (plain files) and `MemStorage` (tests, replay).
- No crate outside `nimk-storage` uses `std::fs` (except `nimk-replay` when writing reports). `arch-lint.toml` enforces this.

## Consequences

- Encryption or compression becomes a decorator, `EncryptedStorage<S: Storage>`, with no change to the engine or replay.
- Each record or blob is an independent unit, which suits per-part encryption.
- One extra `Box<dyn ...>` layer; the log writer runs in the background, so the hot path is unaffected.
- Log queries read segments sequentially; there is no index.
