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

## Newsgroup Selector — Planned

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

The entire prompt line at the bottom should be in inverse text.

Typing the letter beside a newsgroup marks it for reading. A `+` appears
immediately after the letter without shifting the rest of the row. Multiple
groups can be marked before starting.

The list is paginated when its contents do not fit in the available terminal
height.

Keys:

* letter: toggle the corresponding newsgroup's mark
* `q`: quit
* `>`: advance to the next page; do nothing on the last page
* `<`: go to the previous page
* Space:
  * if any groups are marked, start reading them immediately in display order;
  * if no groups are marked and another page exists, advance to that page;
  * on the last page, start reading marked groups in display order;
  * if no groups are marked, behave as though all groups in the group were
    selected and start reading them.

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

For the username column in the thread selector, just list their name, not an email.

The thread article count should be at column 23-25 (inclusive, 1-based.)

* If the username would encroach on this, truncate the username with elipses leaving a space before the thread article count.
* If the thread article count is over 999, list 999 for the count.

Each thread is assigned one letter on the current page. Only the first unread
article in a thread has the letter. Typing that letter marks the thread to be
read; a `+` is added immediately after the letter without shifting the row.
The user may mark multiple threads before starting to read.

Only threads whose complete display fits on the page are normally included.
When a thread would overflow the remaining space, it starts on the next page.
If a single thread is too long to fit on an otherwise empty page, it may be
shown on the current page when at least two lines remain, truncated with
`  ... etc. [X lines total]`, where `X` is the number of unread article lines
omitted.

Keys:

* letter: toggle the corresponding thread's mark
* `q`: quit the app
* `>`: advance to the next page; do nothing on the last page
* `<`: go to the previous page
* Space:
  * if any threads are marked, start reading them immediately in display order;
  * if no threads are marked and another page exists, advance to that page;
  * on the last page, start reading marked threads in display order;
  * if no threads are marked, behave as though all threads in the group were
    selected and start reading them.

The `<` and `>` keys mean the literal characters, not the arrow keys.

The newsgroup name at the top and the entire `Select Threads` prompt line are
rendered in inverse colors. Article rows without a letter are indented one
space farther than the lettered thread row.

## Article View — In progress

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

Keys:

* `q`: return to the thread selector
* Space: show the next article in the thread

## Thread Hierarchy — Planned

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

If an article refers to a parent missing from the loaded overview metadata,
append `(?)` beside that article in the hierarchy. This is an unresolved-parent
marker, not a subject or article-number placeholder. When parent metadata is
loaded, update the hierarchy and remove `(?)`; if it remains unavailable,
retain the marker.
