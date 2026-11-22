# btrn documentation

- [Architecture](architecture.md) is the index and summary of cross-cutting
  module boundaries.
- [Core model](model.md) defines groups, articles, threads, state, and ownership.
- [NNTP protocol](nntp.md) defines server-facing behavior and wire handling.
- [Decoder](decoding.md) defines MIME and charset decoding.
- [Persistence](persistence.md) defines configuration and newsrc storage.
- [Environment](environment.md) defines environment variables that override configuration.
- [Posting](posting.md) defines followup composition and sending.
- [Screen specification](screens.md) describes terminal layouts and interactions.
- [Runtime and commands](runtime.md) defines command dispatch and lifecycle behavior.
- [Testing](testing.md) defines test layers and critical journeys.
- Agent rules for each source path, and the skills, are listed in the
  "Subsystem rules" table in [AGENTS.md](../AGENTS.md#subsystem-rules).

Section status labels describe implementation progress:

- **Implemented**: substantially matches current code.
- **In progress**: partially implemented or has known gaps.
- **Planned**: specified as a target but not implemented yet.
- **not binding**: rationale or open questions; never a requirement.

The specifications in this directory are reference material. Agent instructions remain concise and subsystem-specific.
