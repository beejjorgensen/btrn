---
name: add-config-key
description: "Add, rename, or remove a btrn config.toml key or its environment-variable override. Use whenever a feature needs a new setting, default, or tunable limit."
---

# Adding a config key

Every key lives in several places in `src/config.rs` that must stay in sync,
plus the spec. Missing one gives a key that parses but is ignored, or one that
is rejected as unknown. Follow the
[persistence rules](../../instructions/persistence.instructions.md).

## Procedure

1. Add the field to `pub struct Config`, with a `///` comment if its meaning is not obvious from the name. Use the type the code wants (`Duration`, `Vec<String>`, `usize`), not the TOML type.
2. Set its default in `Config::with_paths`. Every `Config` is built there, so this is the only place defaults live.
3. Add the TOML name to `KNOWN_KEYS` and bump the array length, as in `[&str; 15]`. Unknown-key errors and line numbers depend on this list.
4. Add an `Option<...>` field with the same name to `struct ConfigFile`, using the TOML-facing type:
   * a port uses `NonZeroU16`;
   * seconds use `u64`;
   * a command or editor uses `CommandValue`, which accepts a string or an array.
5. Apply it in `Config::apply_toml` with an `if let Some(...)` block. Convert there, for example `Duration::from_secs`, or `CommandValue::into_words` for commands.
6. If an environment variable overrides it:
   * add a `*_VARIABLES` constant listing the `BTRN_` name first;
   * pass `environment_value(&...)` from both `Config::from_environment` and `Config::load`;
   * extend `Config::apply_environment`.

   Empty variables already count as unset.
7. Wire it to its consumer through `Config`. Pass the value in, for example `overview_limits` in `src/main.rs`, and don't read `Config` from deep inside protocol or model code.
8. Update tests in the `src/config.rs` tests module. Use [the test checklist](./references/checklist.md).
9. Document it:
   * the key, its meaning and its default in the Config list in [docs/persistence.md](../../../docs/persistence.md);
   * any environment variable in [docs/environment.md](../../../docs/environment.md);
   * the behavior it controls in the spec that owns that behavior, linking back to the key.
10. Run `cargo test config`, then `cargo test`, `cargo fmt --check`, and `python3 scripts/check-docs.py`.

## Secrets

If the value can be secret, like `password`, never put it in an error, log, or
`Debug` output, and never write it to disk. Config errors already report only a
category and line number; keep it that way.
