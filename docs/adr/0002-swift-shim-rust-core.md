# 0002. Thin Swift shim, statically linked Rust core over a C ABI

> **Status:** accepted · **Date:** 2026-11-08

## Context

InputMethodKit and AppKit are only convenient from Swift/Objective-C.
The core logic (composer, Telex, policy, trainer, log) needs pure tests, deterministic replay and control over hot-path allocation.
Dropping marked text (committing each character and faking backspaces) is fragile across apps.

## Decision

- One process, `NeuralIMK.app`: a Swift shim plus a statically linked `libnimk_ffi.a`.
- Swift only forwards `NSEvent`/context to Rust and executes `NimkOut`. No decision logic lives in Swift.
- `nimk-ffi` is the only crate allowed `unsafe` and `extern "C"`. Every exported function wraps `catch_unwind` and returns `consumed = false` on failure.
- The header is generated with `cbindgen`. The build uses `cargo` and `swiftc`, not an Xcode project.
- The word being typed is kept as marked text; the preedit underline is accepted.

## Consequences

- All behavior is testable with `cargo test` and replay, without AppKit.
- Changing mode mid-word just replaces the preedit; no fake backspaces.
- One ABI boundary: Rust-owned strings live until the next call, so Swift must copy them immediately.
- The underline always shows under the word being typed; hiding it is out of scope.
- Note: the original "<= 300 lines" size goal for the shim no longer holds (about 870 lines of Swift today); the no-logic rule still does.
