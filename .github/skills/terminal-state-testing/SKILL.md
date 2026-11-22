---
name: terminal-state-testing
description: "Test btrn terminal state, key dispatch, pagination, rendered rows, and Crossterm output without a real terminal, and decide when a pseudo-terminal journey is needed."
---

# Terminal state testing workflow

Almost all terminal behavior is tested without a terminal or socket. Follow
the [terminal rules](../../instructions/terminal.instructions.md).

## Layers and where they live

* **Key dispatch:** call `operation_for_key(screen, key)` in `src/terminal/reader.rs`, or the private `operation_for_key(key)` in `src/terminal/group_selector.rs`, and assert the operation enum. Check that the key is ignored on screens where it doesn't apply.
* **State transitions:** build a `SessionState` with `load_articles` and the test helper `loaded_article(number, id, parent)`, wrap it with `reader_for(&state)`, then call `ReaderState` methods such as `start_reading`, `advance_article`, `parent_target` and `adopt_fetched_parent`. Assert the selected article, the screen, and that `state.is_read` and newsrc are unchanged unless the operation marks read.
* **Rendering:** render into a `Vec<u8>` and assert on the text. There is no snapshot framework; tests compare strings or rows directly.
  * `render_thread_list` and `thread_pages` in `src/terminal/thread_selector.rs`, and `render_article_page`, take explicit width and height.
  * `strip_escapes` in the `src/terminal/article_view.rs` tests removes escape sequences, so you can compare visible text.
  * Check inverse text by looking for the reverse-attribute escape around the expected text.
* **Thread window:** `build_hierarchy` in `src/state/thread_hierarchy.rs` returns a grid of rows and inverse rows that you can assert line by line.

## Procedure

1. Add or change the named operation and its state method first, then the renderer.
2. Test key dispatch, then state transitions, then a representative rendering at one or two terminal sizes. Don't test every size.
3. When rendering depends on the terminal size, test the boundary sizes, such as the 46-column cutoff for the thread window and a page that just fits.
4. Use a pseudo-terminal only for the critical journeys in the [testing specification](../../../docs/testing.md#critical-e2e-journeys--planned). There is no harness in the repo yet, so say in your report if a journey could not be run.
5. Run the narrow module (`cargo test terminal::reader`, `cargo test article_view`), then `cargo test`.

Use the [terminal coverage list](./references/coverage.md) to pick cases.
