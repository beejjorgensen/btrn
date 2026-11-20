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

The group command loads the configured newest-unread overview window, builds
threads, and prints each thread's unread/total count and subject. To print the
previous flat article-number and subject listing instead:

```sh
cargo run -- news.example.net --group comp.test --flat
```

Plaintext is available only when explicitly requested:

```sh
cargo run -- news.example.net --plaintext --group comp.test
```