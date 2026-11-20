# Experiments with Copilot

Designed by Beej, written by LLMs.

## NNTP probe

List groups using implicit TLS:

```sh
cargo run -- news.example.net
```

Select a group and display its server-reported article range:

```sh
cargo run -- news.example.net --group comp.test
```

Plaintext is available only when explicitly requested:

```sh
cargo run -- news.example.net --plaintext --group comp.test
```