# btrn runtime and commands specification

Status labels are defined in the [documentation index](./README.md).

## Async? — Planned

This is an open question. We should discuss pros and cons.

## Command and Feature Expansion — In progress

Key commands and interactive features will expand substantially over time. The implementation should make it easy to add commands and screen behavior without rewriting each screen or scattering key checks across rendering code.

* Define screen-specific key bindings as data or explicit dispatch tables, not as long chains embedded throughout renderer code.
* Translate raw terminal input into named application operations or commands before changing model or screen state.
* Keep key parsing separate from command execution and from terminal rendering.
* A command should have a small, explicit effect on `SessionState`, return a view/state result, or both.
* Screen dispatch should handle only keys and operations that belong to that screen. Global commands such as quit or resize should have a shared mechanism.
* Future commands may include search, marking threads or groups read, sorting, filtering, jumping, refresh, help, and editor integration.
* Search for threads by subject or author and thread selection should operate through model queries or operations rather than direct terminal-text matching.
* Letter selection and reserved keys must be resolved centrally enough that new commands can coexist with generated selection letters.
* Command help, key labels, and key dispatch should be derived from the same binding definitions where practical.
* Invalid or unavailable commands should return a normal UI result or status message; they must not silently do nothing or terminate the app.
* Avoid a general plug-in framework or large command registry. Add focused command handlers or a small binding table as the actual feature set grows.
* Grouped commands should share model operations where possible. For example, marking all threads read should use the same read-state operation as marking one thread read.
* Screen navigation and command execution should remain testable without a real terminal or network connection.


## Screen Specification — In progress

The terminal screen layouts and interactions are specified in
[screens.md](./screens.md).

The headless group-selector model is derived from `LIST ACTIVE` results and
`NewsrcState`. It sorts groups by name, distinguishes subscribed,
unsubscribed, and untracked groups, and keeps temporary read-selection marks
separate from persisted subscription state. Cursor movement clamps at either
end, and marked group names are returned in display order. It does not perform
network requests or terminal rendering.

The non-interactive `--group` command loads the configured newest-unread
overview chunks into `SessionState`, builds the headless thread list, and prints
thread summaries with unread counts. `--flat` retains the article-number and
subject listing for inspecting individual overview rows. The fetch/parse/model
flow is testable against the fake NNTP server without a live terminal.
`--group <name> --thread-root <number>` opens the first unread article in a
loaded thread, falling back to its first article when all are read. It reports
an explicit error if the requested root is outside the configured overview
window. The selected article follows the normal fetch, decode, render, and
post-flush read-marking flow; navigation alone never marks it read.


## Multiple instances — Planned

Disallow multiple instances of trn running at once for now.

## Editor — Planned

This should obey the `BTRN_EDITOR` variable, then the `EDITOR` variable, then just run `vim`.