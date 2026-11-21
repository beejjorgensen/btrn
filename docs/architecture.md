# btrn architecture

This is a from-scratch Rust USENET newsreader.

Status labels are defined in the [documentation index](./README.md).

## Goals

- Keep NNTP transport, protocol parsing, MIME/charset decoding, model/state,
  persistence, threading, and terminal rendering in separate modules.
- NNTP and protocol code must not depend on Crossterm or terminal state.
- Rendering code must not open sockets or read files.
- Prefer small, explicit types and ordinary functions over speculative
  abstractions.
- Apply bounded network timeouts and preserve raw article bytes until decoding.
- Mark articles read only after presentation or through an explicit mark-read
  operation.
- Use the server article number and group as the scope of newsrc state.
- Keep article bodies and thread structures out of persistent storage.

The detailed goals and command-dispatch requirements remain in
[runtime.md](./runtime.md).

## Module boundaries

- `NNTP protocol`: network I/O, command and response parsing, capabilities,
  authentication, and typed protocol failures.
- `Model`: groups, articles, threads, read state, session operations, and
  navigation.
- `Persistence`: configuration and newsrc parsing/writing.
- `Decoder`: MIME, content-transfer encodings, charsets, and text normalization.
- `Rendering`: terminal layout, key-to-operation mapping, and screen state.

Protocol types must not implement terminal rendering. Rendering code must not
read or write files or directly issue NNTP commands.

## Specifications

- [Core model](./model.md): data ownership, groups, articles, threads,
  newsrc state, session state, pagination, and relationships.
- [NNTP protocol](./nntp.md): transport, startup, authentication, response
  parsing, groups, overview, failures, lifecycle, and interoperability.
- [Decoder](./decoding.md): raw article parsing, MIME, charsets, and decoding
  diagnostics.
- [Persistence](./persistence.md): configuration and newsrc storage.
- [Terminal screens](./screens.md): selector and article-view layouts,
  interactions, and key bindings.
- [Runtime and commands](./runtime.md): asynchronous implementation questions,
  command dispatch, CLI behavior, editor, and instance lifecycle.
- [Testing](./testing.md): fixture strategy, test layers, and critical
  pseudo-terminal journeys.

## Cross-cutting decisions

- The synchronous NNTP implementation is the initial target. An asynchronous
  runtime should be adopted only if terminal responsiveness or concurrent I/O
  requires it.
- A single NNTP session exists per application instance.
- Protocol operations return typed success/failure results and do not mutate
  terminal or screen state.
- Session operations mutate state only after an operation succeeds.
- `NewsrcState` is the only persistent article-related state.
