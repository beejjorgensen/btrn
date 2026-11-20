---
description: "Use when implementing or testing NNTP transport, wire parsing, XOVER/overview, authentication, greetings, capabilities, article HEAD/BODY/ARTICLE retrieval, or fake NNTP servers."
applyTo:
  - "src/nntp/**"
  - "src/protocol/**"
  - "tests/**/*nntp*"
  - "tests/fixtures/**/*"
---

# NNTP implementation rules

- Implement implicit TLS with system trust; never silently downgrade a configured TLS connection.
- Serialize one command and one response at a time.
- Bound command lines at 64 KiB and decoded multiline responses at 16 MiB.
- Unstuff exact dot lines and stop only at an exact `.\r\n` terminator.
- Keep status code, full first line, multiline data, and an error category in protocol responses.
- Do not convert NNTP bytes to UTF-8 in transport or protocol parsing.
- Never put passwords or authentication arguments in logs or errors.
- Reconnect once after transport loss; retry temporary `4xx` or `480` authentication challenges at most once.
- Never create a `Group` or cached `Article` after an unsuccessful operation.
- Read protocol fixtures and status-handling details from [architecture](../../docs/architecture.md).
