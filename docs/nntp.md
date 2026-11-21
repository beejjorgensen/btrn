# btrn NNTP specification

Status labels are defined in the [documentation index](./README.md).

## NNTP Behavior — In progress

This section specifies the server-facing behavior for the NNTP protocol implementation. It is independent of rendering and persistence, and should be testable with a fake NNTP server.

### Scope — In progress

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

### Transport — Implemented

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

### Session Startup — In progress

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

### Authentication — In progress

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

### Low-Level Response Handling — Implemented

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

### Group Operations — In progress

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

### Initial Overview Range — In progress

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

### Header and Body Parsing — Implemented
Raw article headers and bodies remain byte-preserving protocol data until the
decoder boundary. Parsing and MIME/charset rules are specified in
[decoding.md](./decoding.md).

### Failure Policy — Planned

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

### Concurrency and Lifecycle — In progress

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

### Logging and Privacy — In progress

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

### Server Interoperability — In progress

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
