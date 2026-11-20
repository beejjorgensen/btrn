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

To open the first unread article from a thread, identify the root article
number from the printed thread summary:

```sh
cargo run -- news.example.net --group comp.test --thread-root 12345
```

The root must be present in the configured newest-unread overview window.

Start the minimal interactive thread reader for a group:

```sh
cargo run -- news.example.net --group comp.test --interactive
```

Use `j`/`k` or the arrow keys to select a thread, Enter or Space to open it,
Space to advance through its articles, and `q` to return to the thread list or
quit from the list. `Backspace` also returns from an article to the list.

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