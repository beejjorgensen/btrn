# Screen Specification

Status labels are defined in the [documentation index](./README.md).

Text inside `[[double square brackets]]` is rendered in inverse colors without
the brackets. Most keystrokes take effect immediately; pressing Return is not
required except where noted.

## Newsgroup and Thread Selector Common — In progress

Selectors can have multiple pages. The footer indicates the current page:

* `All`: all items fit on one page.
* `Top X%`: first page, showing through X% of the items.
* `X%`: a middle page, showing through X% of the items.
* `Bot`: last page.

The selector assigns letters to the visible items on the current page. Letters
used by commands are skipped. Typing an item's letter toggles its mark.

## Newsgroup Selector — Implemented

```
Newsgroups       8 groups

a     6 rec.bicycles.tech
b     1 comp.programming
d    38 comp.lang.c
e    17 comp.os.linux.misc
f     1 alt.folklore.computers
g    21 rec.arts.sf.written
i    68 rec.arts.drwho
j    16 alt.os.linux

-- Select a newsgroup (natural order) -- All --
```

The word `Newsgroups` at the top should be in inverse text.

The entire prompt line should be in inverse text.

There should be one blank line between the `Newsgroups` header at the
top and the list of newsgroups.

There should be one blank line between the end of the list of newsgroups
and the prompt line.

The prompt line should appear one line below that blank line. It should not
be pinned to the bottom of the screen when there is unused space below it.
When the list fills the available space, the prompt may be at the bottom.

Typing the letter beside a newsgroup marks it for reading. A `+` appears
immediately after the letter without shifting the rest of the row. Multiple
groups can be marked before starting.

Letters a-z and digits 0-9 can be used to represent newsgroups. Except that any
letters or digits that are otherwise used to control the newsgroup selector UI
will not be included.

Newsgroups that have no unread threads should not appear in the newsgroup
selector. (Future improvement to keep in mind: a key to toggle
"all"/"unread-only")

Only subscribed newsgroups (those in the `newsrc` file with a `:` after the
name) should be listed in the newsgroup selector.  (Future improvement to keep
in mind: a key to toggle "all"/"subscribed-only".)

The count beside each group is the unread article-number count estimated from
the server's active range and the group's newsrc read ranges.

The list is paginated when its contents do not fit in the available terminal
height.

Keys:

* letter: toggle the corresponding newsgroup's mark
* `q`: quit
* `>`: advance to the next page; do nothing on the last page
* `<`: go to the previous page
* Space:
  * on any page but the last, advance to the next page, whether or not any
    groups are marked;
  * on the last page, start reading the marked groups in display order, or,
    if none are marked, behave as though all subscribed groups shown were selected and
    start reading them.
* Return: if any groups are marked, start reading them immediately in display
  order from any page; if none are marked, do nothing.

The `<` and `>` keys mean the literal characters, not the arrow keys.

## Thread Selector — Implemented

The thread selector is shown for one newsgroup. Each visible thread has a
letter, an unread/article count, a subject, and its article rows:

```
comp.lang.c          38 articles

a fir                   9  >[p.i.c] ships flyin and faitin
  Chris M. Thomasson
  Chris M. Thomasson
  Chris M. Thomasson
  Chris M. Thomasson
  fir
  fir
  fir
  fir
b Keith Thompson        3  >[meta] O tempora, o mores!
  Ross Finlayson
  David Brown
d Tristan Wibberley    26  >Inversion-of-control and dependency-injection in C
  Michael S
  Ross Finlayson
  Tristan Wibberley
  bartc
  Ross Finlayson
  Tristan Wibberley
  bartc
  Michael S
  Ross Finlayson
  bartc
  fir
  Ross Finlayson
  fir
  fir
  fir
  ... etc. [21 lines total]

[[-- Select Threads (date order) -- All --]]
```

The count after the newsgroup name is the number of unread articles in the
listed threads; read articles loaded only for thread context are not counted.

For the username column in the thread selector, just list their name,
not an email.

There should be one blank line between the newsgroup name header at the
top and the list of threads.

There should be one blank line between the end of the list of threads
and the prompt line.

The prompt line should appear one line below that blank line. It should not
be pinned to the bottom of the screen when there is unused space below it.
When the list fills the available space, the prompt may be at the bottom.

The thread article count should be at column 23-25 (inclusive, 1-based),
right justified.

* If the username would encroach on this, truncate the username with
  elipses leaving a space before the thread article count.
* If the thread article count is over 999, list 999 for the count.

Each thread is assigned one letter on the current page. Only the first unread
article in a thread has the letter. Typing that letter marks the thread to be
read; a `+` is added immediately after the letter without shifting the row.
The user may mark multiple threads before starting to read.

Letters a-z and digits 0-9 can be used to represent threads. Except that any
letters or digits that are otherwise used to control the thread selector UI
will not be included.

Articles that have been read should not appear in the thread selector. Once an
article has been read, the next time the thread selector is viewed, that article
will not be present. (Future improvement to keep in mind: a key to toggle
"all"/"unread".)

Only threads whose complete display fits on the page are normally included.
When a thread would overflow the remaining space, it starts on the next page.
If a single thread is too long to fit on an otherwise empty page, it may be
shown on the current page when at least two lines remain, truncated with
`  ... etc. [X lines total]`, where `X` is the number of unread article lines
omitted.

Keys:

* letter: toggle the corresponding thread's mark
* `q`: return to the newsgroup selector (or, in a multi-group reading run,
  finish this group and continue to the next selected group; after the last
  one, the newsgroup selector is shown again)
* `Esc`: quit the app, including from a multi-group reading run
* `>`: advance to the next page; do nothing on the last page
* `<`: go to the previous page
* Space:
  * on any page but the last, advance to the next page, whether or not any
    threads are marked;
  * on the last page, start reading the marked threads in display order, or,
    if none are marked, behave as though all threads in the group were selected and
    start reading them.
* Return: if any threads are marked, start reading them immediately in display
  order from any page; if none are marked, do nothing.

The `<` and `>` keys mean the literal characters, not the arrow keys.

The newsgroup name at the top and the entire `Select Threads` prompt line are
rendered in inverse colors. Article rows without a letter are indented one
space farther than the lettered thread row.

## Newsgroup selector/Thread selector interstitial — Implemented

When the user moves from the Newsgroup selector to the thread selector
and NNTP commands are being issued:

* The screen should be cleared.
* The bottom line of the screen should say "Loading threads" and some
  kind of percentage or progress indicator.
  * The progress indicator should not spawn a new thread or anything
    heavy-handed. If chunks are coming back from the news server
    synchronously, updating the indicator between chunks would be a good
    place to do it.

We shouldn't leave the alternate screen for this.

## Article View — Implemented

The article view has four areas:

1. Filtered article headers in the upper left.
2. A windowed view of the thread hierarchy in the upper right.
3. The decoded article body.
4. A prompt line at the bottom for keyboard input.

```
comp.lang.c #330019 (34 more)                                           --(1)--(1)--(1)+-(1)--[1]--(1)-
From: "Chris M. Thomasson" <chris.m.thomasson.1@gmail.com>                             \-[1]
[1] Re: [p.i.c] ships flyin and faitin                                  --[1]
Date: Thu Nov 19 14:11:11 PST 2026
Lines: 15

On 11/19/2026 12:22 AM, fir wrote:
> fir pisze:
>> this is overally all not hard but its such kind of things that
>> (for some not quite known for me reason) liek turns from normal coding
>> -like normally you cut through and write it - turns into 'research'
>> where you can research it for weeks or months
>>
>> (and im not saying cutthrough or research are better ways, im not sure,
>> maybe some in the middle)
>
> new version
>
> https://www.youtube.com/watch?v=T0f6Hx_KrvI

It looks good to me.
End of art 330019 (of 330052) [npq]
```

The top line shows the group, the server article number, and `(N more)`, the
count of other unread articles in the loaded group; `(N more)` is omitted when
N is zero.

The header area shows only `From`, `Subject`, `Date`, and `Lines`, in that
order, when present. `Lines` falls back to the decoded body's line count when
the header is missing. `Date` is currently shown as the decoded header value
without reformatting. The Subject header is shown as `[N] subject`, where `N`
is the subject's number in the thread hierarchy.

After `v`, the header area instead shows every header as `Name: value` in the
order received, with encoded words decoded. The Subject header has no `[N]`,
and `Lines` appears only if the article has that header. Every header row
wraps with the `+   ` prefix instead of being truncated, at the narrowed width
beside the thread window and at the full width elsewhere. This all-headers
mode lasts until Ctrl-R or until another article is shown.

The top line, headers, one blank line, and body form one continuous display
that is paged as a whole, with the prompt on the line below it. Each Space
scrolls the display up by the number of new rows it reveals, at most one
screenful, so the top line and headers scroll away after the first page. The
last page shows the final screenful of rows, repeating rows already seen when
the remainder is short, so the end-of-article prompt is on the bottom line of
the screen. An article that fits on one screen shows its prompt directly after
its content.
The thread hierarchy window occupies the rightmost 25 columns of the first 6
display rows and scrolls away with the headers. The top line and headers in
those rows are limited to the terminal width minus 26 columns (one column of
gap). A line that would run into the window wraps, and the rest continues on
the next row prefixed with `+   ` in the leftmost column. Wrapped rows push the
following headers and body down, and a continuation below the window rows uses
the full width. If the headers take fewer than 6 rows, blank rows pad the
window area before the body. Terminals narrower than 46 columns omit the
window and use the layout below.
Other header rows are truncated to the terminal width. In the body, tabs expand to
8-column stops, control characters are shown as U+FFFD, and long lines wrap at
the terminal width. While more remains, the inverse prompt is
`--MORE--(NN%)`, where NN is the percentage of the display's rows shown
through the current page. On the last page the prompt is
`End of art N (of M) [npq]`, where M is the group's highest article number
from `GROUP`.

The article is marked read after its first page is presented. Paging, `r`,
Ctrl-R, `v`, and terminal resizes re-render the in-memory article at the new size without
fetching it again or changing read state.

Keys:

* Space: show the next page; on the last page, show the next unread article in
  the reading run, returning to the thread selector after the last one
* `n`: skip to the next unread article
* `p`: go back to the previous unread article in the current thread
* `q`, Backspace, or `Esc`: return to the thread selector
* `r`: redraw
* Ctrl-R: restart the article from its first page with the usual headers, as
  if just arrived
* `v`: restart the article from its first page showing all headers
* `[` or Left: go to the parent of the current article, which may be on an
  earlier row of the thread hierarchy. On a thread root the prompt is followed
  by `There is no parent article prior to this one.`
* `]` or Right: go to the current article's first reply, the next node on its
  row. With no replies the prompt is followed by
  `This is the last leaf in this tree.`
* `(` or Up: go to the nearest node drawn straight above the current article:
  the closest earlier row with a node whose cell overlaps the current cell's
  columns. Connectors are passed over. With none the prompt is followed by
  `This article has no previous sibling.`
* `)` or Down: like `(`, looking at later rows. With none the prompt is
  followed by `This article has no next sibling.`

  These four keys work on every page of the article. When a move lands on a
  `(?)` placeholder, the missing article is fetched by `Message-ID` and added
  to the thread (see [model.md](./model.md)). If the server no longer has it,
  the prompt is followed by `Parent article is not available.` for `[` or
  `Article is not available.` for the others. Each notice lasts until the next
  redraw, and the current article stays selected.
* `F`: post a followup that quotes this article (see [posting.md](./posting.md))
* Ctrl-C: exit the program (on every screen), saving newsrc

## Thread Hierarchy — Implemented

An example full render:

```
[1] [p.i.c] ships flyin and faitin

  (1)+-(1)+-(1)
     |    \-(1)
     |-(1)--(1)--(1)--(1)--(1)--(1)--(1)+-(1)--[1]--(1)--(1)
     |                                  \-(1)
     \-[1]--[1]--[1]--[1]--[1]
```

When multiple subject lines appear in one hierarchy, they are numbered in
order; `1` in the example identifies the subject.

Read articles use square brackets and unread articles use parentheses. The
currently viewed article is entirely inverse, including its brackets or
parentheses. The previously viewed article has only its number inverted.

A node's first reply continues on its row joined by `--`, or by `+-` when it
has more replies. Each later reply starts on a new row below the previous
reply's subtree, joined by `|-`, or by `\-` for the last one, with `|`
continuing down between them. Subjects are compared after stripping leading
`Re:` prefixes. An article without a subject uses its parent's number. The
current article is drawn as read, because presenting it marks it read.

In the article view the hierarchy is shown through a 25-column by 6-row
window. The window centers the current article (horizontally on its cell,
vertically on the third row) when possible. Near the edges of the graph it
stops at the graph's extent, and the highlight moves toward that edge of the
window. A graph smaller than the window is anchored at its top-left.

An article whose parent is missing from the loaded metadata hangs from a
placeholder, drawn as `(?)` and joined like any other node. Placeholders
chain through the `References` ancestors that are also missing, and
replies to the same missing article share its placeholder:

```
(?)+-[1]--(1)
   \-[1]
```

A placeholder has no subject number and is never highlighted. When the
missing article is loaded, its placeholder is replaced by the article; if it
remains unavailable, the placeholder stays.
