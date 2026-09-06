# Task RV-027: Reject Unknown Keys in Configuration Parsing

## Subsystem
`config` (Configuration File Validation)

## Summary
When an operator supplies a `--config` TOML file containing misspelled directives (e.g. `[server]` or `http_pott`), silently ignoring unrecognized keys causes the server to start with default values contrary to operator intent. The configuration parser must collect all unrecognized keys and fail startup with a descriptive error listing the unexpected fields.

## Expected Behavior
1. Any unrecognized top-level key or subsection in the TOML configuration must be caught and returned as `RunvaneError::Config("unknown config key(s): ...")`.
2. Valid configuration files with known keys must parse successfully without errors.

## Files Affected
- `src/config.rs`

## Verification
Run:
```bash
cargo test --lib config::tests
```
Assert that supplying unrecognized keys in TOML causes `Config::from_toml_str` to fail.
