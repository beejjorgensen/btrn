---
name: nntp-fixtures
description: "Build or update fake NNTP servers and wire fixtures. Use for greetings, TLS, capabilities, AUTHINFO, MODE READER, LIST ACTIVE, XOVER/OVER, dot-stuffing, HEAD/BODY/ARTICLE, POST, malformed responses, disconnects, and protocol integration tests."
---

# NNTP fixture workflow

btrn's protocol tests run a scripted fake server on a local socket in the same
test. There is no fixture framework: each test scripts the exact wire exchange.
Follow the [NNTP rules](../../instructions/nntp.instructions.md).

## Building blocks

These are private helpers, copied into each test module that needs them, today
`src/nntp/mod.rs` and `src/group_loader.rs`:

* `fake_server(|mut stream| { ... })` binds `127.0.0.1:0`, accepts one connection on a thread, and returns `(SocketAddr, JoinHandle)`.
* `read_command(&mut reader, "GROUP comp.test")` reads one line and asserts it equals the command plus `\r\n`, so the test fails on any unexpected command.
* `connect(address)` in `src/nntp/mod.rs` opens a plaintext `NntpSession` with short timeouts.

Inside the server closure, wrap a clone in a reader with `let mut reader = BufReader::new(stream.try_clone().unwrap());`, then alternate `read_command` and `stream.write_all(b"...")`.

## Procedure

1. Pick the operation and the status paths to cover, using the [checklist](./references/checklist.md). Cover broadly across the suite; one test need not hit every failure.
2. Script the server in order:
   * the greeting, `200` or `201`;
   * each command the client is expected to send, checked with `read_command`;
   * the response as a byte string with explicit `\r\n` endings.

   Multiline responses end with `.\r\n`, and dot-stuffed lines start with `..`.
3. For loader tests, also script `CAPABILITIES`, `GROUP`, and `LIST OVERVIEW.FMT` before the `OVER` commands. The `OVER` ranges you expect *are* the assertion on request planning.
4. On the client side, drive the real `NntpSession` methods, such as `select_group`, `list_overview` and `article_headers_by_id`, and assert the typed result or the `ArticleError`/`OverviewError` variant.
5. Join the server thread with `server.join().unwrap()`. This fails the test if the client never sent a command the script expects, or if the server panicked.
6. For TLS, follow `connects_over_tls_and_exchanges_a_command` in `src/nntp/mod.rs`. It loads `tests/fixtures/nntp/tls-test-cert.der` and `tls-test-key.der` with `include_bytes!`, serves through `rustls::ServerConnection`, and connects with `NntpSession::connect_with_roots`. Don't generate new certificates unless these expire.
7. Run `cargo test nntp` or the loader module, then `cargo test`.

## Pitfalls

* Write the whole response before the client reads it. A missing `\r\n` or terminator makes the client wait until its read timeout, which looks like a hang.
* Never put real credentials in fixtures; use values like `reader` and `secret`.
