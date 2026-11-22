# btrn architecture

This is a from-scratch Rust USENET newsreader.

Status labels are defined in the [documentation index](./README.md).

## Goals

The rules that apply to every change (module separation, privacy, timeouts,
read marking, and what may be persisted) are in [AGENTS.md](../AGENTS.md).
They are not repeated here.

## Module boundaries

| Module | Path | Responsibility |
| --- | --- | --- |
| NNTP protocol | `src/nntp/` | Network I/O, command and response parsing, capabilities, authentication, `POST`, and typed protocol failures. |
| Group loading | `src/group_loader.rs` | Plans overview requests from newsrc and feeds the rows into session state. |
| Model | `src/model/` | Raw articles, header parsing, Message-IDs, and article metadata. |
| Threading | `src/thread/` | The thread tree, including placeholders for missing parents. |
| State | `src/state/` | Session state, read state, thread list, navigation, and the hierarchy grid. |
| Decoder | `src/decoder/` | MIME, content-transfer encodings, charsets, and text normalization. |
| Persistence | `src/persistence/`, `src/config.rs` | Configuration and newsrc parsing/writing. |
| Posting | `src/posting/` | Composition file, editor hand-off, header checks, and outgoing articles. |
| Rendering | `src/terminal/` | Terminal layout, key-to-operation mapping, and screen loops. |
| CLI | `src/main.rs`, `src/cli.rs` | Argument handling and wiring the modules together. |

## Specifications

- [Core model](./model.md): data ownership, groups, articles, threads,
  newsrc state, session state, pagination, and relationships.
- [NNTP protocol](./nntp.md): transport, startup, authentication, response
  parsing, groups, overview, failures, lifecycle, and interoperability.
- [Decoder](./decoding.md): raw article parsing, MIME, charsets, and decoding
  diagnostics.
- [Persistence](./persistence.md): configuration and newsrc storage.
- [Environment](./environment.md): environment variables that override
  configuration.
- [Posting](./posting.md): followup composition, editor hand-off, header
  checks, and sending.
- [Terminal screens](./screens.md): selector and article-view layouts,
  interactions, and key bindings.
- [Runtime and commands](./runtime.md): command dispatch, CLI entry points,
  instance lifecycle, and open questions.
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
