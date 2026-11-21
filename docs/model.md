# btrn core model specification

Status labels are defined in the [documentation index](./README.md).

## Core Data Model — In progress

The core model should describe content and state independently of NNTP protocol details and terminal rendering. Protocol responses, terminal layouts, and files on disk may be translated into these types, but should not leak into the rest of the application.

Effective configuration and persistent newsrc storage are defined in
[persistence.md](./persistence.md).

### Group — In progress

`Group` represents a newsgroup known to a server.

* `name`: canonical newsgroup name.
* `first_article`: lowest article number reported by the server.
* `last_article`: highest article number reported by the server.
* `posting_status`: posting availability reported by the server.
* `subscription`: whether the newsgroup is subscribed.
* `read_ranges`: ranges of server article numbers already read.

The newsrc subscription status and server article-number ranges are distinct. Article numbers are server-local and may differ between servers or later change as articles expire.

### Article — In progress

`Article` is the stable, application-level representation of an article.

* `message_id`: the globally unique RFC `Message-ID`, without the surrounding angle brackets.
* `subject`: decoded subject text.
* `from`: decoded author value.
* `date`: parsed article date and UTC offset when available.
* `references`: ordered `Message-ID` values from the `References` header.
* `in_reply_to`: optional `In-Reply-To` value.
* `headers`: filtered headers selected for display.
* `body`: decoded MIME text selected for display.
* `server_number`: article number in the currently selected server group, if known.
* `group_name`: the group containing this server number, if known.
* `read`: whether the article has been read.

A single article may have server numbers in multiple groups. It should not be identified only by `subject`, server article number, or a source-specific cache key.

The model should preserve raw article data long enough to support decoding and diagnostics, but normal UI code should consume normalized text and structured headers.

### Thread — In progress

`Thread` represents a hierarchy of articles connected by RFC references.

* `root`: the root article in the thread.
* `articles`: all known articles in the thread.
* `subject`: display subject chosen from the root or earliest displayed article.
* `unread_count`: number of unread articles.
* `newest_date`: newest known article date.

Article relationships should be built from `Message-ID`, `References`, and optionally `In-Reply-To`. Use the last valid `References` ID as the parent candidate, falling back to `In-Reply-To` only when `References` is empty. Subject lines must not create parent-child relationships. Missing, duplicate, cyclic, orphaned, and malformed references require a deterministic fallback; an article with no valid relationship becomes a thread root. Resolve duplicate Message-IDs to the article with the lowest server article number and report the duplicate. Break cycles by removing the parent edge from the highest article number in the cycle. Unresolved parent IDs remain attached to root nodes as diagnostics.

If an article has a reference to a parent `Message-ID` that is not present in the currently loaded overview range, the parent relationship should remain unknown rather than being invented from subject or article position. In the thread hierarchy view, display the article with a literal `(?)` marker to indicate that its parent was not loaded or could not be resolved. The marker should not replace the article's normal read/unread and subject numbering.

When parent metadata is loaded later, replace `(?)` with the resolved relationship. If parent metadata remains unavailable because it expired or was outside the server range, retain `(?)` in the hierarchy. A loaded article may become a root in the current view while it has unresolved references.

The hierarchy should support:

* finding the parent and children of an article;
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

Headless article navigation is created from a loaded thread root and its
group-scoped read ranges. It starts at the first unread article in deterministic
thread traversal order, or the first article if the thread is fully read. Next
and previous operations move across all known articles; next/previous unread
operations skip read articles and do not wrap at either end. Navigation itself
does not mutate read state.

### NewsrcState — Implemented

Persistent subscription and read-range state is specified in
[persistence.md](./persistence.md). The model consumes it but does not own it.

### SessionState — In progress

`SessionState` describes the in-memory state of one running application instance.

* `config`: effective configuration.
* `server_capabilities`: protocol capabilities discovered from the server.
* `groups`: known groups and their subscription/read state.
* `articles`: articles known to the current session, retained only in memory.
* `threads`: threads built from known articles, retained only in memory.
* `active_screen`: group selector, thread selector, or article view.
* `selected_group`: current group selector or thread selector context.
* `selected_thread`: current thread context.
* `selected_article`: article currently shown.
* `pagination`: page state derived from terminal dimensions.
* `layout_width`, `layout_height`: current terminal size.
* `instance_lock`: result of the single-instance lock attempt.

Session state should be changed through explicit operations rather than directly by protocol or rendering code. Loading overview metadata updates the in-memory group/article/thread view but never updates read ranges. Marking an article read is a separate explicit operation keyed by group and server article number; it preserves an existing subscription marker and creates a new entry as unsubscribed rather than implicitly subscribing a group. Suggested operations include:

* `connect`, `disconnect`
* `subscribe_group`, `unsubscribe_group`
* `load_group_range`
* `load_article`
* `mark_article_read`, `mark_thread_read`
* `select_next_article`, `select_previous_article`
* `resize_layout`
* `save_newsrc`

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
* `SessionState` owns runtime state and references model objects.
* `Group` owns its subscription and server range, but read ranges are copied or synchronized from `NewsrcState`.
* `Article` owns article content and references.
* `Thread` owns relationships among known articles.

### Module Boundaries — Implemented

Implementations should preserve these boundaries:

* NNTP protocol: network I/O, command parsing, capability/auth state, and server response types.
* Model: groups, articles, threads, read state, and session operations.
* Persistence: config and newsrc parsing/writing.
* Rendering: terminal layout and key-to-operation mapping.
* Decoder: MIME, content-transfer encoding, charsets, and text normalization.

Protocol types should not implement terminal rendering. Rendering code should not read or write files or directly issue NNTP commands.
