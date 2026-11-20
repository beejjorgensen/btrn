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
- Parse `LIST ACTIVE` rows into typed group listings; preserve unknown posting flags and report malformed rows without discarding valid groups.
- Parse `GROUP` success responses into typed selections; retain server ranges when the reported count disagrees, and treat `411` as an explicit unknown-group failure.
- Never put passwords or authentication arguments in logs or errors.
- When reconnect/retry behavior is implemented, reconnect once after transport loss and retry temporary `4xx` or `480` authentication challenges at most once. Tests for operations that do not exercise retry behavior need not cover these cases.
- Never create a `Group` or cached `Article` after an unsuccessful operation.
- Read protocol fixtures and status-handling details from [architecture](../../docs/architecture.md).
