# Contributing to Foundry

## Local Workflow

Foundry expects every change to keep the repository green with the same contract used in CI.

Recommended local flow:

```bash
make verify
```

Before release preparation:

```bash
make verify-release
```

## Verification Targets

- `make fmt-check`: formatting check
- `make test`: all targets, examples, and acceptance suites
- `make fixture-check`: explicit blueprint and plugin fixture checks
- `make clippy`: clippy with `-D warnings`
- `make package-check`: package dry-run
- `make verify`: format, tests, clippy, and fixture checks
- `make verify-release`: full verification plus package dry-run

## Git Hooks

Enable the project pre-commit hook (auto-regenerates API surface docs when source files change):

```bash
git config core.hooksPath .githooks
```

This is a one-time setup per clone. The hook only runs when `src/`, `Cargo.toml`, or macro crate files are staged.

## API Surface Docs

The `docs/api/` directory is auto-generated from `cargo doc` HTML output. Regenerate manually:

```bash
make api-docs
```

The tool auto-discovers all public modules — no manual registration needed when adding new modules. If the pre-commit hook is enabled, this happens automatically.

## Contribution Expectations

- Keep the public API strongly typed. Do not reintroduce raw semantic strings where typed identifiers or enums already exist.
- Preserve the thin-app consumer model from the blueprint.
- Prefer updating examples, docs, and acceptance fixtures alongside public API changes.
- If you touch bootstrap or registry behavior, keep both fixture families green:
  - `tests/fixtures/blueprint_app`
  - `tests/fixtures/plugin_consumer_app`

## Documentation Expectations

- Keep the README quick-start aligned with the current typed API.
- Update `CHANGELOG.md` for user-visible behavior or workflow changes.
- Update the release checklist if the verification or publish flow changes.
