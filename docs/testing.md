# btrn testing specification

Status labels are defined in the [documentation index](./README.md).

## Testing — In progress

E2E testing should be feasible and valuable, but it should be layered so that terminal automation is used only for a small number of critical journeys.

### Test Layers — In progress

1. **NNTP protocol integration tests**
   * Run a small fake NNTP server in the test suite.
   * Cover greetings, TLS mode when practical, authentication, XOVER, dot-stuffing, `HEAD`, `BODY`, `ARTICLE`, and temporary/permanent errors.
   * Use realistic wire fixtures for malformed responses and transport failures.

2. **Wire-to-model tests**
   * Feed fake-server fixtures through protocol parsing and into groups, articles, threads, and read state.
   * Verify unknown parents produce `(?)` placeholders, read-context rows attach replies to already-read parents, and a parent fetched by Message-ID replaces its placeholder.
   * Verify the configured newest-unread overview window and on-demand older-range loading.

3. **Terminal state and snapshot tests**
   * Separate key handling and screen-state transitions from Crossterm rendering.
   * Test pagination, selection, terminal resizing, marked groups and threads, and article navigation without requiring a live terminal.
   * Use snapshots for representative rendered frames and layout behavior.

4. **Full terminal E2E tests**
   * Launch the real binary in a pseudo-terminal against the fake NNTP server.
   * Send keystrokes through a terminal test harness.
   * Assert screen text and, when practical, inverse rendering state.
   * Keep this layer small because terminal dimensions and escape sequences can make tests brittle.

### Critical E2E Journeys — Planned

This is the only list of pseudo-terminal journeys; agent instructions and skills link here. The initial full-terminal E2E suite should cover:

* first launch and authentication;
* selecting and marking multiple groups;
* entering a large group;
* navigating a paginated thread list;
* opening an article and moving through its thread;
* placeholder `(?)` parents, and moving to a parent with `[` when it is loaded, fetched by Message-ID, or no longer on the server;
* terminal resize;
* quitting and verifying newsrc updates;
* reopening and confirming read ranges.

State operations and key handling must not require a live socket or terminal. Protocol, model, decoder, and rendering boundaries should make it possible to test each layer independently.

Do not test every protocol or rendering combination through the full terminal. Use unit, fixture, and state tests for breadth, and reserve pseudo-terminal E2E tests for a few high-value journeys.

The intended balance between these layers is in [AGENTS.md](../AGENTS.md#approximate-testing-balance).
