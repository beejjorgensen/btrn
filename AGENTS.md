# btrn agent instructions

This is a from-scratch Rust USENET newsreader. The product specification index is
in [docs/architecture.md](docs/architecture.md), with subsystem details in
the linked documents in [docs/](docs/).

## Always apply

- Keep NNTP transport, protocol parsing, MIME/charset decoding, model/state, persistence, threading, and terminal rendering in separate modules.
- NNTP protocol code must not depend on Crossterm or terminal state. Rendering code must not open sockets or read/write files.
- Prefer explicit small types and ordinary functions. Do not add traits, plugins, dependency-injection containers, or configuration solely for hypothetical reuse.
- Avoid new third-party crates unless needed. Crossterm is the intended terminal library; Tokio is not required for the initial synchronous implementation.
- Security and privacy are mandatory: never log or persist passwords, usernames intended as sensitive, authentication arguments, or article bodies. The one exception is the temporary posting composition file specified in `docs/posting.md`. A failed `POST`'s server response text may be shown to the user but never logged.
- Apply bounded timeouts to network operations. Do not silently fall back from TLS to plaintext.
- Preserve typed protocol failures and raw article bytes until the appropriate decoder/model boundary.
- Make state and key-handling transitions testable without a live socket or terminal.
- Mark article read only after presentation in article view or through an explicit mark-read operation.
- Use the server article number and group for newsrc state. Never persist article bodies, overview rows, headers, or thread structures (except the temporary posting composition file).
- Keep the full specification in `docs/`, not in always-loaded agent instructions.

## Rust conventions

- Format Rust with `cargo fmt`.
- Run the smallest relevant test target first, then the full suite when the change is local.
- Add focused unit tests at module boundaries and integration-style fixtures for protocol wire responses and article decoding.
- Prefer deterministic behavior and explicit errors over silent defaults or fallbacks.
- Preserve existing public behavior; gate intentional behavior changes with tests.
- Do not introduce async or a runtime until terminal responsiveness or concurrent I/O requires it.

## Approximate testing balance

- Aim for roughly 60% protocol/model/decoder tests, 25% terminal state and snapshot tests, and 15% critical pseudo-terminal end-to-end journeys. Treat these proportions as a planning guide, not a required test-count target.

Use the fake-server protocol fixtures for broad coverage. Reserve real pseudo-terminal automation for authentication, group/thread navigation, unresolved parents, resize, quit/newsrc, and reopen/read-range journeys.

## Before finishing

- Inspect the diff for unintended files or unrelated changes.
- Run `cargo fmt --check` and the relevant Cargo test targets.
- Report validation failures explicitly with commands and concise failure details.
