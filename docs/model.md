# btrn core model specification

Status labels are defined in the [documentation index](./README.md).

## Core Data Model — In progress

The core model should describe content and state independently of NNTP protocol details and terminal rendering. Protocol responses, terminal layouts, and files on disk may be translated into these types, but should not leak into the rest of the application.

Effective configuration and persistent newsrc storage are defined in
[persistence.md](./persistence.md).

### Current types — Implemented

These are the types the code has today. Prefer extending them over adding the
planned types below.

| Type | Path | Holds |
| --- | --- | --- |
| `GroupSelection` | `src/nntp/group.rs` | A `GROUP` result: name, count, first and last article numbers. |
| `RawArticle` | `src/model/article.rs` | Header fields and body lines as bytes, as received. |
| `MessageId` | `src/model/article.rs` | A normalized Message-ID, without angle brackets. |
| `ArticleMetadata` | `src/model/article.rs` | Message-ID, `References`, `In-Reply-To`, raw `From`/`Subject`/`Date`, and parse issues. Built from overview rows or headers. |
| `LoadedArticle` | `src/state/mod.rs` | An `ArticleMetadata` with its server article number. |
| `ThreadTree` | `src/thread/mod.rs` | Article nodes, placeholder nodes (`MissingNode`), ordered roots, and `ThreadIssue` diagnostics. Nodes are addressed by `ThreadRef::{Article(u64), Missing(MessageId)}`. |
| `GroupState` | `src/state/mod.rs` | One loaded group: metadata by article number and its `ThreadTree`. |
| `SessionState` | `src/state/mod.rs` | `NewsrcState` plus loaded groups by name. |
| `ThreadList`, `ThreadRow` | `src/state/thread_list.rs` | The headless thread-selector view model. |
| `ArticleNavigation` | `src/state/article_navigation.rs` | The article order of one thread and the current position. |
| `DecodedArticle` | `src/decoder/mod.rs` | Display headers and body text after MIME and charset decoding. |

Decoded article bodies exist only while an article is displayed; they are not
stored in `SessionState`.

### Planned types — Planned

These describe the intended direction, not code to write now. Add a field only
when a feature needs it.

* `Group`: a server group with posting status, subscription, and read ranges
  copied from `NewsrcState`. Today this is split between `GroupSelection`,
  `LIST ACTIVE` results, and `NewsrcState`.
* `Article`: decoded subject, author, parsed date and UTC offset, filtered
  headers, and decoded body for display, with its server number and group.
  A single article may have server numbers in multiple groups. It must not be
  identified only by subject, server article number, or a source-specific
  cache key.
* `SessionState` additions: server capabilities, the active screen and
  selections, pagination, terminal size, and the single-instance lock. Today
  screen state lives in the terminal module (for example `ReaderState` in
  `src/terminal/reader.rs`).

The model should preserve raw article data long enough to support decoding and
diagnostics, but normal UI code should consume normalized text and structured
headers.

### Threads — Implemented

Article relationships should be built from `Message-ID`, `References`, and optionally `In-Reply-To`. Use the last valid `References` ID as the parent candidate, falling back to `In-Reply-To` only when `References` is empty. Subject lines must not create parent-child relationships. Missing, duplicate, cyclic, orphaned, and malformed references require a deterministic fallback; an article with no valid relationship becomes a thread root. Resolve duplicate Message-IDs to the article with the lowest server article number and report the duplicate. Break cycles by removing the parent edge from the highest article number in the cycle. Unresolved parent IDs are kept on the article as diagnostics.

If an article's parent `Message-ID` is not in the loaded metadata, the
relationship is never invented from subject or article position. Instead the
article becomes a child of a placeholder node for that `Message-ID`. The
placeholder's own parent comes from the earlier `References` entries: each
earlier ID is the parent of the next, becoming another placeholder when it is
not loaded, until a loaded article is reached. An `In-Reply-To` parent has no
known ancestors. Articles that reference the same missing ancestors therefore
share one thread. When articles disagree about a placeholder's parent, the
lowest-numbered article wins, and a link that would create a cycle is dropped.
Children are ordered by the lowest article number at or below them.

A placeholder holds no metadata and is never read, unread, or navigable by
next/previous. It is drawn as `(?)` in the thread hierarchy (see
[screens.md](./screens.md)). A thread whose root is a placeholder is
identified by its first loaded article in walk order.

When the missing article's metadata is loaded later, its placeholder is
replaced by the article. If it stays unavailable because it expired or was
outside the server range, the placeholder remains.

Two mechanisms load parent metadata:

* When a group loads, overview rows for the newest read articles are loaded as
  well, only to give unread articles their already-read parents and siblings.
  They change no read state and do not add threads to the thread selector,
  which shows only threads with unread articles. The count is
  `context_overview_limit` in [persistence.md](./persistence.md).
* In the article view, a thread-window move (see [screens.md](./screens.md))
  that lands on a placeholder fetches that article by `Message-ID` (see
  [nntp.md](./nntp.md)) and adds it to the loaded group, under the article
  number from its `Xref` header.

The hierarchy should support:

* finding the parent and children of an article or placeholder;
* walking a thread in display order;
* finding the next and previous unread article;
* marking articles read;
* updating an existing thread when newly loaded articles arrive.

The headless thread-list view model is derived from one loaded group and its
newsrc read ranges. It provides one row per thread with a decoded display
subject, article numbers, unread count, newest parsed date, and unresolved-parent
diagnostic. Rows are ordered by newest date first, then newest server article
number and root article number for deterministic fallback. Selection starts at
the first unread thread (or the first row when all are read), supports explicit
index selection, and clamps next/previous movement at the list boundaries.
This view model does not perform terminal rendering or network I/O.

Headless article navigation is created from a loaded thread and its
group-scoped read ranges. It starts at the first unread article in deterministic
thread traversal order, or the first article if the thread is fully read. Next
and previous operations move across all known articles; next/previous unread
operations skip read articles and do not wrap at either end. Navigation itself
does not mutate read state.

### NewsrcState — Implemented

Persistent subscription and read-range state is specified in
[persistence.md](./persistence.md). The model consumes it but does not own it.

### Session operations — In progress

Session state should be changed through explicit operations rather than directly by protocol or rendering code. Loading overview metadata updates the in-memory group/article/thread view but never updates read ranges. Marking an article read is a separate explicit operation keyed by group and server article number; it preserves an existing subscription marker and creates a new entry as unsubscribed rather than implicitly subscribing a group. Current operations include `SessionState::load_overview`, `load_articles`, and `mark_article_read`, and `group_loader::load_group_threads`. Planned ones include subscribing and unsubscribing, marking a whole thread read, and loading older ranges.

Article and thread objects must not be written to disk. The newsrc file is the only article-related persistent state and stores subscription and read ranges only. Article bodies, decoded text, headers, overview rows, and thread structures are discarded when the application instance exits.

Within one session, an article may be retained in memory for navigation or de-duplication, but memory retention is not persistent cache behavior. Protocol retrieval should be used when an object is not already in the current session state. A memory limit or eviction policy may be added later, but eviction must not alter read state or persisted newsrc state.

### Read and Selection State — Implemented

Read state is derived from `NewsrcState.read_ranges` and the current server number for each article in a group. An article selected for reading but not yet rendered should not be marked read merely by loading metadata.

An article is marked read only after it has been selected and presented in article view, or by an explicit mark-read operation. This timing should be deterministic and testable.

The initial command-line article view accepts `--group <name> --article <number>`. It retrieves headers and body, renders the article, and marks it read only after the terminal output has been flushed successfully. The updated newsrc is then saved atomically.

### Pagination State — In progress

Pagination is derived layout data, not article or thread state. It should contain:

* the current page;
* the first and last logical items on that page;
* which items are complete, truncated, or omitted;
* enough information to refresh the page after terminal resizing.

The same thread or article should not contain screen-specific layout fields.

### Relationships — In progress

The intended ownership is:

* `Config` provides settings and resolved credentials.
* `NewsrcState` owns persisted subscription and read-range state.
* `SessionState` owns `NewsrcState` and the loaded groups.
* `GroupState` owns a loaded group's metadata and its `ThreadTree`.
* `ThreadTree` owns relationships among loaded articles and placeholders.
