# btrn USENET newsreader

This is a from-scratch USENET newsreader in Rust.

## General Goals

* Minimal reliance on 3rd-party crates.
  * NNTP client code should be from scratch.
  * Decisions should be made for every potential crate pulled it.
  * Common crates (for example rand, though that won't be needed for the newsreader) are more acceptable than unpopular crates.

* Exceptions for minimal reliance on crates:
  * Crossterm is what we should use.
    * Avoid higher-level terminal code such as Ratatui.
  * Tokio if we want async.

* Keep the code highly modular, reusable, and easy to maintain.
  * Separate protocol, decoding, model/state, persistence, threading, and terminal rendering concerns into coherent modules.
  * NNTP transport and response parsing, MIME/content-transfer decoding, charset decoding, newsrc parsing, and thread construction should have useful interfaces that do not depend on the terminal UI.
  * Prefer small, named types and functions with explicit inputs and outputs over shared mutable state or long argument lists.
  * Public modules should expose only what their callers need. Protocol modules should not know about Crossterm, and renderer modules should not open network connections or read files.
  * Design reusable components around stable concepts: byte streams, protocol responses, decoded text, message references, read ranges, and page layout.
  * Module reuse is for maintenance and testability, not for hypothetical crates.io publication or a generic framework.
  * Avoid unnecessary abstractions, plugin systems, dependency-injection containers, trait hierarchies, or configuration for single implementations.
  * Do not add an abstraction until a real second use case or a clear test boundary requires it.
  * Add a trait or interface when it creates a useful seam for tests or isolates an unavoidable external dependency; otherwise, prefer ordinary functions and types.
  * Organize code by responsibility, not by a rigid number of files or directories. A module may contain a few closely related types and functions.
  * Include focused unit tests at module boundaries and integration-style fixtures for NNTP wire responses and article decoding.

## Async?

This is an open question. We should discuss pros and cons.

## Command and Feature Expansion

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

## Testing

E2E testing should be feasible and valuable, but it should be layered so that terminal automation is used only for a small number of critical journeys.

### Test Layers

1. **NNTP protocol integration tests**
   * Run a small fake NNTP server in the test suite.
   * Cover greetings, TLS mode when practical, authentication, XOVER, dot-stuffing, `HEAD`, `BODY`, `ARTICLE`, and temporary/permanent errors.
   * Use realistic wire fixtures for malformed responses and transport failures.

2. **Wire-to-model tests**
   * Feed fake-server fixtures through protocol parsing and into groups, articles, threads, and read state.
   * Verify unknown parents produce `(?)`.
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

### Critical E2E Journeys

The initial full-terminal E2E suite should cover:

* first launch and authentication;
* selecting and marking multiple groups;
* entering a large group;
* navigating a paginated thread list;
* opening an article and moving through its thread;
* unresolved-parent and `(?)` behavior;
* terminal resize;
* quitting and verifying newsrc updates;
* reopening and confirming read ranges.

State operations and key handling must not require a live socket or terminal. Protocol, model, decoder, and rendering boundaries should make it possible to test each layer independently.

Do not test every protocol or rendering combination through the full terminal. Use unit, fixture, and state tests for breadth, and reserve pseudo-terminal E2E tests for a few high-value journeys.

A reasonable initial coverage balance is:

* roughly 60% protocol/model/decoder tests;
* roughly 25% terminal state and snapshot tests;
* roughly 15% end-to-end pseudo-terminal journeys.

## Core Data Model

The core model should describe content and state independently of NNTP protocol details and terminal rendering. Protocol responses, terminal layouts, and files on disk may be translated into these types, but should not leak into the rest of the application.

### Config

`Config` is the effective configuration after environment-variable and configuration-file handling.

* `server`: NNTP server host name.
* `port`: server port.
* `use_tls`: whether to use TLS.
* `username`, `password`: optional authentication values.
* `password_command`: optional structured command and argument list used to obtain a password. The executable and each argument are stored literally; no shell is invoked.
* `editor`: effective editor command.
* `config_dir`: normally `~/.config/btrn`.
* `newsrc_path`: normally `~/.config/btrn/newsrc`.
* `connect_timeout`: NNTP TCP/TLS connection timeout in seconds. Default: `10`.
* `read_timeout`: NNTP socket read/write inactivity timeout in seconds. Default: `30`.
* `initial_overview_limit`: maximum number of newest unread article numbers to load when initially entering a group. Default: `10,000`.
* `overview_chunk_size`: maximum range requested in one XOVER call. Default: `1,000`.

Passwords should be read at connection time and should not be written to disk. Password resolution should produce either a clear authentication error or a usable password.

The initial configuration file is `~/.config/btrn/config.toml` and accepts `server`, `port`, `use_tls`, `username`, `password`, `password_command`, `editor`, `connect_timeout`, `read_timeout`, `initial_overview_limit`, and `overview_chunk_size`. Strings use TOML basic-string syntax and `password_command` is an array of strings. The CLI host, port, and `--plaintext` option override their file values; `BTRN_EDITOR` then `EDITOR` override the file's editor. Credentials are configured only in the file in this initial version. Configure exactly one of `password` and `password_command` when `username` is set. The password command is executed directly, without a shell, only when credentials are resolved.

For example:

```toml
server = "news.eternal-september.org"
username = "your-account-name"
password_command = ["pass", "show", "usenet/eternal-september"]
```

After capability discovery, configured credentials trigger authentication even if the server does not list `AUTHINFO`. `AUTHINFO USER` status `281` completes authentication without a password; `381` sends `AUTHINFO PASS`; `480` is returned as an authentication challenge. Authentication failures report status categories only and never include credential arguments or server response text.

### Group

`Group` represents a newsgroup known to a server.

* `name`: canonical newsgroup name.
* `first_article`: lowest article number reported by the server.
* `last_article`: highest article number reported by the server.
* `posting_status`: posting availability reported by the server.
* `subscription`: whether the newsgroup is subscribed.
* `read_ranges`: ranges of server article numbers already read.

The newsrc subscription status and server article-number ranges are distinct. Article numbers are server-local and may differ between servers or later change as articles expire.

### Article

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

### Thread

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

### NewsrcState

`NewsrcState` is the persistent cross-session state for subscription and read-article information.

* `entries`: newsrc entries keyed by canonical group name.
* `subscribed`: whether the entry is subscribed.
* `read_ranges`: normalized, non-overlapping ranges of server article numbers.

Newsrc parsing must distinguish a missing entry, a subscribed entry, and an unsubscribed entry. Range parsing must handle reversed ranges, duplicate ranges, out-of-order ranges, open-ended data, and malformed input without silently corrupting existing state.

Updates should be based on server article numbers, but article numbers from different groups must be stored separately. Writes to disk should use a same-directory temporary file, flush it, and atomically replace the destination. Persist only subscription markers and read ranges; article metadata and bodies must never be serialized. Preserve comments or formatting only if a round-trip-preserving parser is added.

### SessionState

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

### Read and Selection State

Read state is derived from `NewsrcState.read_ranges` and the current server number for each article in a group. An article selected for reading but not yet rendered should not be marked read merely by loading metadata.

An article is marked read only after it has been selected and presented in article view, or by an explicit mark-read operation. This timing should be deterministic and testable.

The initial command-line article view accepts `--group <name> --article <number>`. It retrieves headers and body, renders the article, and marks it read only after the terminal output has been flushed successfully. The updated newsrc is then saved atomically.

### Pagination State

Pagination is derived layout data, not article or thread state. It should contain:

* the current page;
* the first and last logical items on that page;
* which items are complete, truncated, or omitted;
* enough information to refresh the page after terminal resizing.

The same thread or article should not contain screen-specific layout fields.

### Relationships

The intended ownership is:

* `Config` provides settings and resolved credentials.
* `NewsrcState` owns persisted subscription and read-range state.
* `SessionState` owns runtime state and references model objects.
* `Group` owns its subscription and server range, but read ranges are copied or synchronized from `NewsrcState`.
* `Article` owns article content and references.
* `Thread` owns relationships among known articles.

### Module Boundaries

Implementations should preserve these boundaries:

* NNTP protocol: network I/O, command parsing, capability/auth state, and server response types.
* Model: groups, articles, threads, read state, and session operations.
* Persistence: config and newsrc parsing/writing.
* Rendering: terminal layout and key-to-operation mapping.
* Decoder: MIME, content-transfer encoding, charsets, and text normalization.

Protocol types should not implement terminal rendering. Rendering code should not read or write files or directly issue NNTP commands.

## NNTP Behavior

This section specifies the server-facing behavior for the NNTP protocol implementation. It is independent of rendering and persistence, and should be testable with a fake NNTP server.

### Scope

The initial implementation should support:

* secure NNTP using an implicit TLS connection;
* plaintext NNTP only as an explicitly configured fallback;
* server greetings and capability discovery;
* group listing and selection;
* article metadata retrieval;
* filtered header and body retrieval;
* optional authentication;
* protocol errors and connection recovery;
* temporary, in-memory NNTP response data.

Article bodies and decoded article content must not be persisted. All article content should remain in memory for the lifetime of the current application session and be discarded on exit.

The initial implementation should not require posting, NNTP compression, streaming extensions, or asynchronous cancellation.

### Transport

The protocol implementation should use CRLF line endings on the wire and should exchange only complete NNTP lines for commands.

* Default to TLS for new configurations.
* Use port `563` when `use_tls` is true and no explicit port is configured.
* Use port `119` when `use_tls` is false and no explicit port is configured.
* Never silently fall back from a configured TLS connection to plaintext.
* Keep the TLS failure separate from authentication failure.
* Apply a `10`-second connect timeout and a `30`-second socket read/write inactivity timeout by default.
* These values should be configurable through `Config`; no timeout should be unlimited.
* The read timeout is applied independently to each read or write operation rather than as one lifetime limit for an entire response.
* One NNTP command and its response should be serialized at a time.

The server greeting is the first response after connecting:

* `200` means posting is allowed.
* `201` means posting is prohibited.
* Any other greeting is a connection-level failure.

For secure connections, TLS must be established before reading the server greeting. Certificate verification should use the system trust store. Explicitly permitting a disabled certificate check is not an initial requirement.

### Session Startup

After the greeting, use this order:

1. Send `MODE READER`, unless the greeting or server capability is known to make that inappropriate.
2. Send `CAPABILITIES`.
3. Parse and retain server capabilities.
4. Authenticate if the configuration supplies credentials or a password command.
5. Verify or refresh the active reader state.

`MODE READER` is preferred over assuming a transit-only session. A server may reject it or omit it in unsupported configurations; a non-fatal rejection must be distinguishable from a connection or authentication failure. The app must not claim support for an unsupported mode.

Capabilities should be stored as protocol state, not as server-specific booleans scattered through the application. At minimum, recognize:

* `MODE-READER`;
* `AUTHINFO`;
* `READER`;
* `XOVER` and `OVER`;
* `HDR`;
* `STARTTLS`;
* `TLS`;
* `LIST ACTIVE`;
* `LISTGROUP`;
* `COMPRESS DEFLATE`.

Compression and STARTTLS are out of scope for implicit-TLS startup. A server requiring plaintext STARTTLS must not be upgraded unless that behavior is deliberately added.

### Authentication

Authentication should occur only after capability or greeting state indicates that NNTP auth is needed or requested.

For username/password authentication:

* send `AUTHINFO USER <username>`;
* send `AUTHINFO PASS <password>` only after the server accepts the username;
* never include the password in logs or errors;
* treat `281` as success;
* treat `381` after `AUTHINFO USER` as a prompt for `AUTHINFO PASS`;
* treat `480` as an authentication challenge;
* treat `481`, `482`, `502`, and `503` as authentication or authorization failures.

If a password command is configured:

* execute the configured executable with the configured arguments only when credentials are needed;
* capture stdout as the password;
* remove a trailing newline;
* reject empty output;
* reject command start failure and non-zero exit status with an authentication-error result;
* never write the resolved password to config or newsrc.

Username and password values must be passed to NNTP as protocol arguments without shell parsing. The `password_command` array must be passed as an executable plus literal arguments. Shell expansion, pipelines, wildcards, command substitution, and redirection are not supported. If shell behavior is requested in the future, it must be a separate, explicitly named, opt-in feature; it must not be represented by `password_command`.

An unauthenticated server may expose only limited groups. This is a normal server state, not an automatic fatal error. A requested operation may still fail with `480`, in which case it should prompt or re-run authentication and then retry at most once.

### Low-Level Response Handling

Every response should be represented by:

* a numeric status code;
* the full first-line text;
* optional multiline data;
* and a protocol error category.

Status ranges should be classified:

* `1xx`: informational or continuation;
* `2xx`: success;
* `3xx`: continuation required;
* `4xx`: temporary server or network protocol failure;
* `5xx`: permanent server or command failure.

Connection loss, malformed numeric response, missing terminator, truncated data, and invalid multiline framing are transport/protocol errors, not group or article not-found errors.

Text-line reads must enforce a maximum line size of `64 KiB` (65,536 bytes). This limit prevents unbounded memory use and distinguishes oversized input from ordinary server text.

Multiline NNTP responses must enforce a maximum total decoded size of `16 MiB` (16,777,216 bytes). This includes dot-unstuffed data bytes and line ending bytes, but excludes the terminating `.\r\n` marker. Responses exceeding this limit must produce `ProtocolError::OversizedResponse` and must close the connection rather than attempting to continue parsing.

Multiline reads must stop at an exact line containing only `.`. Dot-stuffed lines beginning with `..` must have one leading dot removed. A dot encoded as `..` is not a data marker.

UTF-8 decoding must not occur before the model or decoder boundary. NNTP text and headers may contain charset-specific bytes and malformed sequences.

### Group Operations

Start a group by sending `GROUP <name>` and process `211`:

* `<count> <first> <last> <group>`
* if count is inconsistent with the reported range, retain the range and mark count inconsistent for diagnostics.

A `411` response means that the group is unknown or unavailable. No `Group` entry should be created as if the server operation succeeded.

For group enumeration, prefer `LIST ACTIVE` over issuing repeated `GROUP` commands.

Parse `LIST ACTIVE` each line as:

* `<group-name> <last> <first> <status>`;
* `y`: posting allowed;
* `m`: moderated;
* `n`: posting prohibited.

`LIST ACTIVE` wildmat syntax should be avoided unless a focused group search is added. Unknown status values should be preserved or normalized as unknown rather than mapped to posting prohibited.

When metadata for a group is needed:

* use `XOVER <start>-<end>` first when the server capability says `XOVER` or `OVER` is available;
* use `LISTGROUP <name> <start>-<end>` or `HDR` only if overview is unavailable and a complete article-number strategy is still needed;
* never infer article presence solely from an overview row.

XOVER should accept server-provided field names and fallback to standard NNTP field positions only when `XOVER` output is unnamed. The initial standard field order should be:

1. `Subject`
2. `From`
3. `Date`
4. `Message-ID`
5. `References`
6. `Bytes`
7. `Lines`

For each overview row:

* article number must be a decimal integer;
* missing or malformed optional fields should not discard the article number;
* `Message-ID` should be normalized by removing only surrounding `<` and `>` characters;
* `References` should be split into ordered, normalized message IDs;
* malformed `References` entries may be preserved for diagnostics but should not create parent relationships;
* rows outside the requested range should be ignored;
* duplicate article numbers should resolve to the last complete row;
* no header field may be used to prove message identity except `Message-ID`.

### Initial Overview Range

Large groups can contain hundreds of thousands or millions of article numbers. Loading every overview row at startup would make startup slow, consume substantial memory, and produce more metadata than the user can view. Loading only a tiny fixed range may hide older unread threads and make navigation incomplete.

For the initial implementation:

* determine unread server article numbers for the selected group from `NewsrcState`;
* configure the number of newest unread overview rows to load in `initial_overview_limit`;
* default `initial_overview_limit` to `10,000`;
* split requests into ranges no larger than `overview_chunk_size`, defaulting to `1,000`;
* perform older-range loads only when the user pages beyond the currently loaded range or explicitly requests a refresh;
* include the surrounding known article range when required to complete thread roots and parent relationships, then discard or release transient rows that are no longer needed for the active session;
* never infer that articles outside the loaded range are read; they are simply not loaded.

Range sizes have these tradeoffs:

* **Entire unread range**
  * Pros: complete thread construction and navigation are available immediately; no lazy-loading surprises.
  * Cons: can be extremely slow and memory-heavy for active groups, can exceed the multiline response limit, and can delay startup for days or weeks.
* **Small range, such as 100–500 articles**
  * Pros: very fast startup and low memory use.
  * Cons: usually splits threads, hides many unread subjects, and requires frequent network operations while paging.
* **Medium range, such as 1,000–10,000 articles**
  * Pros: small enough for bounded startup work while usually covering a useful amount of recent activity; 10,000 rows fit the initial 16 MiB response limit in ordinary overview responses.
  * Cons: still insufficient for unusual or high-volume groups; threads may extend outside the range and may need additional metadata.
* **Time-based range**
  * Pros: avoids server numbering range issues and gives a predictable recent history.
  * Cons: requires parsing dates and sorting overview rows, and a slow or irregular server may have no usable overview date for some rows.

The recommended default is `10,000` newest unread articles, requested in `1,000`-article chunks. This is a compromise that bounds startup, supports practical thread pages, and can be expanded or loaded older as the user navigates. Article numbers in a group are not assumed to be continuous; unavailable articles are skipped, and a requested range may return fewer rows.

Retrieve full metadata or bodies with `HEAD`, `BODY`, or `ARTICLE`.

* `HEAD <article-number>` for filtered headers;
* `BODY <article-number>` for the body;
* `ARTICLE <article-number>` only when a complete RFC 5322-style article is needed;
* retain server article number and group outside the article text.

Status `423` means unknown article number. `430` means unknown article or no article with that Message-ID. These statuses must remain distinguishable from network and malformed-response failures.

Do not retain `423`, `430`, or permanent server failures as article content. Such responses may appear only in transient operation results and must not create or update a cached article.

### Header and Body Parsing

Full article parsing should occur after a successful multiline response:

* split at the first blank line;
* unfold continuation lines;
* preserve malformed header lines in parse diagnostics;
* preserve duplicate fields in wire order and use the last case-insensitive match for a single-value lookup;
* preserve header names, values, and body lines as bytes until the decoder boundary;
* extract typed `Message-ID`, `References`, and `In-Reply-To` values while retaining diagnostics for malformed identifiers;
* decode RFC 2047 encoded words at the decoder boundary;
* parse `References`, `In-Reply-To`, and `Message-ID` into normalized message IDs;
* preserve filtered display headers separately from raw headers.

MIME handling is not part of the NNTP protocol module:

* feed the raw header/body representation to the decoder;
* select text alternatives using MIME type and charset;
* decode quoted-printable and base64;
* normalize line endings and terminal-facing text after decoding.

The initial decoder supports `text/plain`, multipart text alternatives, RFC 2047
encoded display headers, quoted-printable and base64 transfer encodings, and
recognized character sets. It returns typed diagnostics for malformed or
unsupported content rather than silently treating it as successfully decoded.
The raw article remains unchanged; callers choose how to report decoder issues.

### Failure Policy

Protocol operations should report typed failures:

* `TransportError`: connect, TLS, socket read/write, timeout, or abrupt close;
* `ProtocolError`: malformed status, invalid multiline framing, or oversized data;
* `ServerTemporaryError`: status `4xx`;
* `ServerPermanentError`: status `5xx`;
* `AuthenticationError`;
* `GroupNotFound`;
* `ArticleNotFound`;
* `NotAuthenticated`;
* `UnsupportedOperation`.

The UI should receive an operation result and decide whether to show a message or transition screens. Protocol code must not alter terminal state.

The default failure policy is:

* reconnect once after a clean or abrupt transport loss;
* do not retry permanent `5xx` or malformed protocol responses;
* retry a server temporary `4xx` no more than once;
* retry a `480` authentication challenge once after authentication;
* stop immediately on a second authentication failure;
* reconnecting must repeat greeting, capabilities, and authentication startup;
* selection or read-state mutations must not occur before a successful operation result.

### Concurrency and Lifecycle

Only one NNTP session should exist per application instance.

The NNTP session should own:

* the active socket;
* greeting and capability state;
* authentication state;
* the currently selected group;
* an operation/result command sequence;
* the last protocol error.

The session should expose operations with explicit enter/leave semantics:

* `connect`;
* `disconnect`;
* `list_active_groups`;
* `select_group`;
* `list_overview`;
* `fetch_headers`;
* `fetch_body`;
* `fetch_article`.

Async implementation is deferred. A synchronous implementation may block for bounded network operations. Tokio or another asynchronous runtime may be adopted only if it becomes necessary for terminal responsiveness or concurrent I/O.

### Logging and Privacy

Logs should contain status codes and command names, not full passwords or article bodies.

Logs may contain:

* server host;
* status code;
* command category;
* elapsed time;
* article/group names;
* a safe representation of malformed response data.

Logs must not contain:

* passwords;
* username if that is considered sensitive;
* raw authentication arguments;
* private article bodies;
* user configuration secrets.

### Server Interoperability

The initial target is Eternal-September. Before assuming full compatibility:

* support standard `LIST ACTIVE` and XOVER/overview behavior;
* tolerate server-specific `MODE READER` and greeting behavior;
* tolerate absent optional capabilities;
* preserve unknown overview fields rather than rejecting them;
* retain unknown server status for diagnostics;
* do not rely on server article numbers being contiguous;
* do not rely on overview ordering or local-message headers;
* treat expired articles and changed server ranges as normal.

The protocol implementation should include fixtures for successful and malformed responses to:

* greeting;
* capabilities;
* authentication;
* group selection;
* group listing;
* overview;
* article head/body;
* dot-stuffed multiline data;
* timeout and disconnect.

## Screen Specification

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

## Config

We need a config file where we can set up the server to connect to and passwords.

* This should support reading password with the Unix `pass` program.

* Needs to support secure NNTP (test target is going to be eternal-september.)

Configs should go in `~/.config/btrn`.

We also need a `newsrc` file with the following format:

```
newsgroup: already-read-list
newsgroup! already-read-list
```

* `:` means we're subscribed.
* `!` means we're not subscribed.

The already read list is a ranges list. Example:

```
comp.risks: 1-918
comp.os.linux.misc: 1-100585
comp.lang.forth! 1-127386,127391
alt.folklore.computers: 1-236928,236930-2369322,236930
rec.arts.sf.written: 1-639923
```

## Multiple instances

Disallow multiple instances of trn running at once for now.

## Editor

This should obey the `BTRN_EDITOR` variable, then the `EDITOR` variable, then just run `vim`.