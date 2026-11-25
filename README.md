# btrn - Another trn

Directed by Beej Jorgensen <beej@beej.us>; written by Claude and
Copilot.

This is an experiment to gain experience with agentic coding. *Use at
your own risk.* See `LICENSE.md` for details.

I chose this project because I like trn but it didn't support UTF-8 or
various content transfer encodings. And the existing ancient crufty code
was just too terrible to contemplate. And it seemed like a good size to
tackle without going crazy. (Old C version is about 50K loc.)

This is a "from-scratch" implementation made without use of the original
source. The UI is a clone.

***This program is feature-incomplete!***

***Everything is subject to change!***

***Not all paths have been human-tested!***

## Quick Start

```
mkdir -p ~/.config/btrn
echo 'alt.test:' > ~/.config/btrn/newsrc
```

Also add a `~/.config/btrn/config.toml` file. Here are a couple
examples. Replace Eternal September with your server.

Using `pass` for the password lookup:

```
server = "news.eternal-september.org"
username = "loginname"
password_command = "pass show news.eternal-september.org/loginname"
display_name = "AI Bro"
display_email = "aibro@example.com"
```

If you don't use `pass`, you can hardcode a password in the config with
`password` instead of `password_command` if you like to live
dangerously:

```
server = "news.eternal-september.org"
username = "loginname"
password = "4"
display_name = "AI Bro"
display_email = "aibro@example.com"
```

Then launch with:

```
cargo run -- news.eternal-september.org --interactive
```

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
