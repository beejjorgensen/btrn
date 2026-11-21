# btrn decoding specification

Status labels are defined in the [documentation index](./README.md).

### Header and Body Parsing — Implemented

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
