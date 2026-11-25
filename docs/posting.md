# btrn posting specification

Status labels are defined in the [documentation index](./README.md).

## Posting — Implemented

If the user is at the article view prompt (either the `MORE` prompt or
the end-of-article prompt), they can hit `F` (capital `F`) to post a
followup to the article being viewed.

Posting never changes read state or newsrc.

Configuration keys are described in [persistence.md](./persistence.md),
and the environment variables that override them in
[environment.md](./environment.md).

## Followup Without Quoting — Planned

Lowercase `f` will post a followup without quoting the article. The same
composition flow will also be used to post new, non-reply threads.
Details to be specified.

## Default Editor

If the editor is not specified in the config file or environment, it
shall default to `vim`.

## Posting Check

Upon hitting `F` from the article viewer, first check that posting is
allowed. Each of the following refuses posting with its own message:

| Condition | Message |
|---|---|
| The server greeting was `201` | `This server does not allow posting.` |
| `CAPABILITIES` is available and does not list `POST` | `This server does not support posting.` |
| The current group's `LIST ACTIVE` status is `n` | `Posting to <group> is not allowed.` |

The group status is fetched with `LIST ACTIVE <group>` for the current
group only; other groups in the followup's `Newsgroups` are not checked.

The message is shown on a cleared alternate screen, followed by
`Press any key to continue.` Any key returns the user to the
end-of-article prompt.

A `201` reply to `MODE READER` also counts as the server not allowing
posting, and a `200` reply allows it.

## Warning Screen

If posting is allowed, the user will be presented with these three
paragraphs of warning text:

```
(Be sure to double-check the attribution against the signature, and trim
the quoted article down as much as possible.)

This program posts news to thousands of machines throughout the entire
civilized world.  Your message will cost the net hundreds if not
thousands of dollars to send everywhere.  Please be sure you know what
you are doing.

Are you absolutely sure that you want to do this? [ny]
```

If the followed-up-on article has `Followup-To: poster`, add this
paragraph before the final question:

```
The author requested replies by email (Followup-To: poster).
```

The text should cleanly word-wrap at the terminal width.

The cursor should appear after the `[ny]` text with a space in between.

It is not necessary to hit return after entering `n` or `y`.

If the user answers anything other than `y` (case-sensitive), they
immediately arrive at the end-of-article prompt, even if they pressed
`F` from the `MORE` prompt.

If the user enters `y` (case-sensitive), they begin the posting process.

## Moderated Groups — Planned

A moderated group has `LIST ACTIVE` status `m`. The client posts to it
with an ordinary `POST`; the news server, not the client, forwards the
article to the moderator by email instead of publishing it. No special
client handling is required to post.

Planned: when the current group is moderated, add this paragraph to the
warning screen:

```
<group> is moderated; your article will be mailed to the moderator.
```

## The Composing Process

A temporary file containing the following headers and content should be
constructed and brought up in the editor.

The file is created in `$TMPDIR` (falling back to the platform temporary
directory) with a random name ending in `.txt`, and with permissions
readable and writable only by the user (mode `0600`). It is deleted when
the followup is sent or aborted, when the editor cannot be launched, and
when the program exits, including through Ctrl-C.

> Although we are not persisting article bodies, this is a valid
> exception to that rule.

```
Newsgroups: example.group.name
Subject: Re: Previous Subject
Followup-To:
From: Example Name <user@example.com>
Organization: Yoyodyne Industries
Keywords:

In article <[article id here]>, Poster <poster@example.com> wrote:
>Lorem ipsum lorem ipsum lorem ipsum lorem ipsum lorem ipsum lorem
>Lorem ipsum lorem ipsum lorem ipsum lorem ipsum lorem ipsum lorem
>Lorem ipsum lorem ipsum lorem ipsum lorem ipsum lorem ipsum lorem
>Lorem ipsum lorem ipsum lorem ipsum lorem ipsum lorem ipsum lorem
```

### Newsgroups

The `Newsgroups` line is populated from the `Followup-To` header of the
article being followed-up on, if it has one. Otherwise it is populated
from that article's `Newsgroups` header, a comma-delimited list of
newsgroups.

If the followed-up-on article has `Followup-To: poster`, use its
`Newsgroups` header instead (and show the warning paragraph above). btrn
does not send email.

The followup is posted to whatever the `Newsgroups` line says after
editing.

### Subject

The `Subject` line should be `Re: ` followed by the followed-up-on
article's decoded subject. If that subject already begins with `Re:`
(case-insensitive), do not add an additional `Re:`. Only `Re:` counts as
an existing prefix.

If the followed-up-on article has no subject, use `Re: [No subject]`.

### From

The `From` line is populated from the config values:

| `display_name` | `display_email` | `From` line |
|---|---|---|
| set | set | `From: Example Name <user@example.com>` |
| unset | set | `From: user@example.com` |
| set | unset | `From: Example Name` |
| unset | unset | `From:` |

A value that is empty or only whitespace counts as unset. The last two
forms have no address and fail the header check unless the user adds one
in the editor.

### Organization

The `Organization` line should be populated from the config/env values,
or, if those values are missing, should be left blank like so:

```
Organization:
```

### Attribution

The article body begins with an attribution line constructed from the
followed-up-on article. `Poster` is that article's decoded `From`
header, verbatim.

| Message-ID | From | Attribution |
|---|---|---|
| present | present | `In article <id>, Poster wrote:` |
| missing | present | `Poster wrote:` |
| present | missing | `In article <id>, Anonymous wrote:` |
| missing | missing | `Anonymous wrote:` |

If the attribution line exceeds 72 characters (counted in characters,
regardless of terminal width), split it right after the comma. The
continuation line is not indented. If either part is still longer than
72 characters, leave it; the user can adjust it in the editor.

### Quoted body

The decoded body of the followed-up-on article is quoted after the
attribution line. Trailing blank lines are trimmed first. Then each line
is prefixed with `>` without regard to whether it was already quoted, so
existing `>` marks are pushed farther right and blank lines become `>`.
The signature is quoted like any other text.

If the body is empty, the attribution line is followed by a single `>`
line.

## After the Editor Exits

All of the following are drawn on a cleared alternate screen. The
terminal leaves the alternate screen and raw mode while the editor runs
and restores them afterward.

* If the editor cannot be launched, print an error (followed by
  `Press any key to continue.`) and return the user to the
  end-of-article prompt after a key press.
* If the editor exits with a non-zero status, print an error and show the
  `Edit or Abort` menu.
* If the file is unchanged, print
  `Warning: the article was not changed.`, run the header check, and
  show the menu chosen by the header check.
* Otherwise, run the header check and show the menu it selects.

## Follow-up headers

First read the headers from the temporary composition file. Header lines
that begin with whitespace continue the previous header (folding). Ignore
any headers that do not have values (or whose values are purely
whitespace).

Any other non-empty header the user entered, such as `Followup-To` or
`Keywords`, is posted as written, except the protected and rejected
headers below.

Header values copied into the composition file from the followed-up-on
article (`Newsgroups`, `Subject`) or from the configuration have every
control character, including line breaks, replaced with a space. Decoded
RFC 2047 text can contain line breaks, and without this a remote article
could add header lines, such as `Control: cancel`, to the followup.

Proper NNTP headers must be produced:

```
From: "Example Name" <user@example.com>
Subject: Re: Previous Subject
Newsgroups: example.group.name
References: <first@example> ... <parent@example>
Message-ID: <random@example.com>
Date: Sun, 22 Nov 2026 14:11:11 -0800
User-Agent: btrn
```

If the organization is specified in the composition file, include an
`Organization` header, else don't include one.

```
Organization: Yoyodyne Industries
```

btrn always sets these headers itself, or leaves them for the server to
add, and silently drops any the user entered: `Message-ID`, `Date`,
`Path`, `User-Agent`, `References`, `In-Reply-To`, `MIME-Version`, every
header whose name begins with `Content-`, and the server-added `Xref`,
`Lines`, `Bytes`, `Injection-Info`, `Injection-Date`,
`NNTP-Posting-Host`, `NNTP-Posting-Date`, `X-Trace`, and
`X-Complaints-To`. `Path` is left for the server to add.

Headers that change what the article does rather than what it says are
rejected by the header check instead: `Control`, `Also-Control`,
`Supersedes`, `Approved`, and `Cancel-Key`. btrn does not send control
messages, cancels, supersedes, or moderator approvals.

The posted headers are written in this order: `From`, `Newsgroups`,
`Subject`, `Followup-To` (if any), `References` (if any), `Message-ID`,
`Date`, any other headers the user entered (such as `Organization` and
`Keywords`) in file order, `User-Agent`, `MIME-Version`, `Content-Type`,
and `Content-Transfer-Encoding`. Spaces around the commas in
`Newsgroups` and `Followup-To` are removed.

### From encoding

An ASCII display name is always written as an RFC 5322 quoted-string,
with `"` and `\` inside it escaped with a backslash. A display name
containing non-ASCII characters is instead written as RFC 2047
encoded-words, without quotes, since encoded-words may not appear inside
a quoted-string. A `From` line without a display name is posted as the
bare address.

### Non-ASCII headers

Non-ASCII text in `Subject` and other unstructured headers is written
as RFC 2047 base64 encoded-words using UTF-8, each at most 75
characters, with one encoded-word per folded header line.

### Message-ID

btrn generates the `Message-ID` as `<unique@domain>`, where `unique` is
random and `domain` is the domain of the final `From` address (normally
from `display_email`), falling back to the local host name. It is
generated once per composition and stays the same across edits and
resends, so a resend cannot create a duplicate article.

### Date

btrn generates the `Date` header in RFC 5322 format when the article is
sent.

### References

`References` is the followed-up-on article's `References` followed by its
`Message-ID`. If the header line would exceed 998 bytes, trim it as in
RFC 5537 section 3.4.4, keeping the first message ID and as many of the
most recent IDs as fit. If the followed-up-on article has no
`Message-ID`, omit `References`.

### Content type and encoding

Let's go with these hardcoded for now:

```
MIME-Version: 1.0
Content-Type: text/plain; charset=utf-8
Content-Transfer-Encoding: 8bit
```

Later we might make that more dynamic or configurable.

## Header sanity/error check

The composition is checked for correctness and completeness:

* `Newsgroups` is present and each comma-separated group name is a valid
  newsgroup name with no whitespace.
* `Subject` is present and not blank.
* `From` is present and parses as an address.
* Every header line has a valid `Name:` form, and no header appears more
  than once.
* No header line contains a control character other than tab.
* No rejected header (`Control`, `Also-Control`, `Supersedes`,
  `Approved`, `Cancel-Key`) has a value.
* A blank line separates the headers from the body.
* No header or body line exceeds 998 bytes, excluding the line ending.
* No ASCII header exceeds 998 bytes once its continuation lines are joined,
  because it is posted on one line. Non-ASCII headers are refolded as
  encoded words instead.
* The body is valid UTF-8.

If some of the headers are not correct or something is missing, print
an error message or messages (note the blank line above and below):

```

ERROR: the "Newsgroups:" line is missing from the header.
ERROR: the "Subject:" line is missing from the header.
ERROR: the "From:" line does not contain a valid address.
ERROR: the "Date:" line appears more than once.
ERROR: btrn does not send the "Control:" header.
ERROR: line 3 contains a control character.
ERROR: line 14 is longer than 998 bytes.
ERROR: the "Keywords:" line is longer than 998 bytes once its continuation lines are joined.
ERROR: the article body is not valid UTF-8.

```

Then proceed to the finalization menu, the content of which will vary
based on the success or failure of the sanity check.

## Sending

The article is sent with `POST` as described in [nntp.md](./nntp.md).
Lines are sent with CRLF line endings; a trailing CR in an LF-terminated
file is not doubled. Any line beginning with `.` has an extra `.` added
(dot-stuffing).

On success (`240`), the user returns to the end-of-article prompt.

On failure (`440`, `441`, or a transport error), print the error and show
the `Send, Edit, or Abort` menu. The error may include the server's
response text, with control characters replaced and limited to one line.
Resending uses the same `Message-ID`. POST is never retried
automatically.

## Finalization Menu

### If All Headers are Correct and Complete

Present the following menu line to the user, noting the blank line
above it:

```

Send, Edit, or Abort [sea]?
```

Pressing return is not necessary after typing `s`, `e` or `a`.

If the user presses an invalid key, do nothing. `Esc` is an invalid key
here.

`s` will post the follow-up message, and return the user to the
end-of-article prompt of the article they are following-up to.

`e` will reenter the editor with the same temporary file they'd edited
earlier.

`a` will abort the posting, discarding the follow-up, and return the
user to the end-of-article prompt of the article they are following-up
to.

Returning to the end-of-article prompt moves the article view to its
last page, even if `F` was pressed from the `MORE` prompt. The article
does not need to be redrawn; only the prompt is shown.

### If Some Headers Incorrect or Missing

The Finalization Menu is modified to behave in the following ways:

* `s` (Send) will **not** be an option in this case.

* The prompt will be (note the blank line above it):

   ```

   Edit or Abort [ea]?
   ```

The `a` and `e` keys behave the same in this case as they do in the case
of all headers being valid.

Pressing `s` in this case should have no effect.

This menu is also shown after the editor exits with a non-zero status.

## Ctrl-C

Ctrl-C exits the program from any screen, including the posting
screens. It behaves like quitting with `Esc` from the reader: newsrc is
saved, the terminal is restored, and the temporary composition file is
deleted. While the editor is running, Ctrl-C belongs to the editor.
