# Config key test checklist

In the tests module of `src/config.rs`:

- `defaults_use_secure_bounded_nntp_and_overview_settings`: assert the new default.
- `parses_supported_config_values_and_quoted_comments`, or a focused new test: a file value overrides the default.
- `unset_keys_keep_defaults`: still passes with the key absent.
- `rejects_out_of_range_numbers_and_duplicate_keys`: covers an out-of-range or wrong-type value, if the type has limits.
- `reports_each_error_category_with_its_line`: still passes. A missing `KNOWN_KEYS` entry shows up here as an unknown-key error.
- `config_errors_never_echo_config_contents`: extend it if the value could be secret.
- `environment_overrides_editor_and_organization`, or a sibling test: the environment wins over the file, and an empty variable is ignored.
- A consumer test at the module that uses the value, for example a `group_loader` fake-server test for an overview limit.
