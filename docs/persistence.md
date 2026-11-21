# btrn persistence specification

Status labels are defined in the [documentation index](./README.md).

## Configuration

### Config — In progress

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

The initial configuration file is `~/.config/btrn/config.toml` and accepts `server`, `port`, `use_tls`, `username`, `password`, `password_command`, `editor`, `connect_timeout`, `read_timeout`, `initial_overview_limit`, and `overview_chunk_size`. The file is parsed as standard TOML with the `toml` crate, so any TOML string form, comments, and multi-line arrays are accepted; `password_command` is an array of strings, `port` must be 1-65535, and timeouts and overview sizes are non-negative integers. Only these top-level keys are accepted: unknown keys and tables are errors, and a repeated key is an error rather than overriding the earlier value. Configuration errors report only a category (syntax, unknown key, or invalid value) and line number; they never include config text or values, since the file may contain a password. The CLI host, port, and `--plaintext` option override their file values; `BTRN_EDITOR` then `EDITOR` override the file's editor. Credentials are configured only in the file in this initial version. Configure exactly one of `password` and `password_command` when `username` is set. The password command is executed directly, without a shell, only when credentials are resolved.

For example:

```toml
server = "news.eternal-september.org"
username = "your-account-name"
password_command = ["pass", "show", "usenet/eternal-september"]
```

After capability discovery, configured credentials trigger authentication even if the server does not list `AUTHINFO`. `AUTHINFO USER` status `281` completes authentication without a password; `381` sends `AUTHINFO PASS`; `480` is returned as an authentication challenge. Authentication failures report status categories only and never include credential arguments or server response text.


## Newsrc state

### NewsrcState — Implemented

`NewsrcState` is the persistent cross-session state for subscription and read-article information.

* `entries`: newsrc entries keyed by canonical group name.
* `subscribed`: whether the entry is subscribed.
* `read_ranges`: normalized, non-overlapping ranges of server article numbers.

Newsrc parsing must distinguish a missing entry, a subscribed entry, and an unsubscribed entry. Range parsing must handle reversed ranges, duplicate ranges, out-of-order ranges, open-ended data, and malformed input without silently corrupting existing state.

Updates should be based on server article numbers, but article numbers from different groups must be stored separately. Writes to disk should use a same-directory temporary file, flush it, and atomically replace the destination. Persist only subscription markers and read ranges; article metadata and bodies must never be serialized. Preserve comments or formatting only if a round-trip-preserving parser is added.

The initial newsrc format is:

```text
newsgroup: already-read-list
newsgroup! already-read-list
```

`:` means subscribed and `!` means unsubscribed. The read list contains
normalized article-number ranges, for example:

```text
comp.risks: 1-918
comp.os.linux.misc: 1-100585
comp.lang.forth! 1-127386,127391
alt.folklore.computers: 1-236928,236930-2369322,236930
rec.arts.sf.written: 1-639923
```
