---
name: nntp-fixtures
description: "Build or update fake NNTP servers and wire fixtures. Use for greetings, TLS, capabilities, AUTHINFO, MODE READER, LIST ACTIVE, XOVER/OVER, dot-stuffing, HEAD/BODY/ARTICLE, malformed responses, disconnects, and protocol integration tests."
---

# NNTP fixture workflow

## Procedure

1. Identify the protocol operation and status path to cover.
2. Add a small fake-server fixture that writes CRLF on the wire and returns realistic first lines.
3. Cover success and each relevant failure category: transport, protocol framing, temporary `4xx`, permanent `5xx`, group-not-found, article-not-found, and authentication.
4. Keep multiline fixtures exact, including dot-stuffed lines and the terminating dot.
5. Wire the fixture through protocol parsing and, when relevant, into model types.
6. Assert status, multiline data, typed errors, and connection closure behavior.
7. Run the narrow protocol test target, then the relevant protocol/model integration suite.

Use [NNTP fixture checklist](./references/checklist.md) for coverage.
