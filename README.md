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

Start with the interactive newsgroup selector:

```sh
cargo run -- news.example.net --interactive
```

Mark groups by typing their letters or digits (except reserved `q`). Only
subscribed groups with estimated unread articles are listed. `<` and `>` page
backward and forward; Space advances through pages, starts marked groups, or
starts all listed groups when pressed on the last page with none marked. Each
selected group opens its thread selector in display order. The thread selector
shows unread articles only and Space advances to the next unread article. In a
multi-group run, `q` advances to the next selected group, then returns to the
newsgroup selector when the batch is finished; `Esc` quits the application.
`q` on the newsgroup selector also quits.

Type a thread's letter to mark it. `<` and `>` page backward and forward;
Space starts marked threads immediately. If none are marked, Space advances
pages and starts all threads when pressed on the last page. In an article,
Space advances to the next article and `q` or `Backspace` returns to the thread
list. `q` quits from the thread list.

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