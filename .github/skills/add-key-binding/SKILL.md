---
name: add-key-binding
description: "Add or change a keystroke on a btrn screen (newsgroup selector, thread selector, or article view). Use for any new command key, rebinding, or key that shows a message."
---

# Adding a key binding

Keys are translated to named operations before anything changes, and the
selectors give out letters and digits to rows. A new key therefore touches
dispatch, the screen loop, letter assignment, tests, and the screen spec.
Follow the [terminal rules](../../instructions/terminal.instructions.md).

## Where keys live

| Screen | Operation enum and mapping | Handled in |
| --- | --- | --- |
| Thread selector and article view | `ReaderOperation`, `operation_for_key(ReaderScreen, KeyEvent)` in `src/terminal/reader.rs` | the `match` in `run_interactive_in_session` |
| Newsgroup selector | `GroupSelectorOperation`, `operation_for_key(KeyEvent)` in `src/terminal/group_selector.rs` | that file's screen loop |
| Posting screens | `src/terminal/posting.rs` | that file |

Ctrl-C (`crate::terminal::is_interrupt`) and `Esc` are handled before the
screen-specific arms. Don't rebind them per screen.

## Procedure

1. Check [docs/screens.md](../../../docs/screens.md) for the key on that screen and on the others. A key may mean different things on different screens, but it must not shadow an existing binding on its own screen.
2. Add a variant to the operation enum, with a `///` comment naming the key and its effect when that isn't obvious.
3. Map it in `operation_for_key` with a `(screen, KeyCode::...)` arm, placed before the catch-all letter arm.
4. Put the behavior in a `ReaderState` (or selector-state) method that takes `&SessionState` and needs no terminal or socket. Network work goes in a separate function the loop calls, like `fetch_parent_metadata`.
5. In the screen loop, call that method and set `redraw` only when something visible changed. For a one-line message, set `reader_state.notice`; it is shown after the prompt on the next redraw and then cleared.
6. **Letters and digits on a selector screen collide with row letters.** If the key is alphanumeric on the thread or newsgroup selector, in either case:
   * exclude it in `selection_letter` in `src/terminal/selector.rs`;
   * add it to the guard in that screen's toggle arm (`letter != 'q'`), since those arms lowercase what they receive, so `Q` collides with `q`;
   * update `selection_letters_skip_q`, `thread_selector_uses_digits_after_available_letters` (in `src/terminal/thread_selector.rs`), and `letters_and_digits_select_group_rows_after_q_is_skipped`.

   On the article view, letters are free.
7. Never mark an article read from a key handler unless the operation is an explicit mark-read; presentation marks read on its own.
8. Test, in the `reader.rs` or `group_selector.rs` tests module:
   * dispatch: the key maps to the operation on its screen and to `Ignore` elsewhere, as in `bracket_moves_to_a_loaded_parent_or_names_the_missing_one`;
   * the state method: its result, plus unchanged read state and newsrc where relevant.

   See the `terminal-state-testing` skill.
9. Document the key in that screen's **Keys** list in [docs/screens.md](../../../docs/screens.md). If the screen's prompt lists keys, as `End of art N (of M) [npq]` does, decide whether the new key belongs there.
10. Run `cargo test terminal`, then `cargo test`, `cargo fmt --check`, and `python3 scripts/check-docs.py`.
