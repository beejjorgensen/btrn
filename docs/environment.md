# btrn environment variables

Status labels are defined in the [documentation index](./README.md).

## Environment Variables — Planned

Environment variables override the corresponding values in the
configuration file described in [persistence.md](./persistence.md). A
variable that is set but empty is treated as unset.

The app should look for the editor in the following environment
variables in this preferred order:

1. `BTRN_EDITOR`
2. `EDITOR`

If neither is set, the config file's `editor` key is used, then the
default `vim` (see [posting.md](./posting.md)).

The app should look for the organization in the following environment
variables in this preferred order:

1. `BTRN_ORGANIZATION`
2. `ORGANIZATION`

If neither is set, the config file's `organization` key is used.
