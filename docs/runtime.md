# btrn runtime and commands specification

Status labels are defined in the [documentation index](./README.md).

## Command and Feature Expansion — In progress

Key commands and interactive features will expand substantially over
time. The implementation should make it easy to add commands and screen
behavior without rewriting each screen or scattering key checks across
rendering code.

* Define screen-specific key bindings as data or explicit dispatch
  tables, not as long chains embedded throughout renderer code.
* Translate raw terminal input into named application operations or
  commands before changing model or screen state.
* Keep key parsing separate from command execution and from terminal
  rendering.
* A command should have a small, explicit effect on `SessionState`,
  return a view/state result, or both.
* Screen dispatch should handle only keys and operations that belong to
  that screen. Global commands such as quit or resize should have a
  shared mechanism.
* Future commands may include search, marking threads or groups read,
  sorting, filtering, jumping, refresh, help, and editor integration.
* Search for threads by subject or author and thread selection should
  operate through model queries or operations rather than direct
  terminal-text matching.
* Letter selection and reserved keys must be resolved centrally enough
  that new commands can coexist with generated selection letters.
* Command help, key labels, and key dispatch should be derived from the
  same binding definitions where practical.
* Invalid or unavailable commands should return a normal UI result or
  status message; they must not silently do nothing or terminate the
  app.
* Avoid a general plug-in framework or large command registry. Add
  focused command handlers or a small binding table as the actual
  feature set grows.
* Grouped commands should share model operations where possible. For
  example, marking all threads read should use the same read-state
  operation as marking one thread read.
* Screen navigation and command execution should remain testable without
  a real terminal or network connection.


## CLI and Headless Selector Models — In progress

Interactive layouts and keys, including the multi-group reading run and `q`
and `Esc` on each screen, are specified only in [screens.md](./screens.md).

The headless group-selector model is derived from `LIST ACTIVE` results and
`NewsrcState`. It sorts groups by name, distinguishes subscribed,
unsubscribed, and untracked groups, and keeps temporary read-selection marks
separate from persisted subscription state. Cursor movement clamps at either
end, and marked group names are returned in display order. It does not perform
network requests or terminal rendering.

Command-line entry points:

* `--interactive` opens the newsgroup selector; `--interactive --group <name>`
  opens that group's thread selector directly, and `q` there exits to the
  shell.
* `--group <name>` without `--interactive` loads the group's overview chunks
  (see [nntp.md](./nntp.md)) into `SessionState`, builds the headless thread
  list, and prints thread summaries with unread counts. `--flat` prints the
  article number and subject of each overview row instead.
* `--group <name> --thread-root <number>` opens the first unread article in a
  loaded thread, falling back to its first article when all are read. It
  reports an explicit error if the requested root is outside the loaded
  overview window. The selected article follows the normal fetch, decode,
  render, and post-flush read-marking flow; navigation alone never marks it
  read.

These flows are testable against the fake NNTP server without a live terminal.

## Multiple instances — Planned

Disallow multiple instances of btrn running at once for now.

## Posting and Editor — Implemented

See [posting.md](./posting.md) and [environment.md](./environment.md).

## Open questions — not binding

* Whether to adopt an asynchronous runtime. The current rule is in
  [AGENTS.md](../AGENTS.md): stay synchronous until terminal responsiveness
  or concurrent I/O requires otherwise.
