# Foundry Release Checklist

Use this checklist for `0.1.x` release preparation.

## Before Versioning

1. Review `CHANGELOG.md` and move finished release notes out of `Unreleased`.
2. Confirm `README.md` still matches the current typed public API and examples.
3. Confirm release docs and workflows still reflect the actual verification contract.

## Version Bump

1. Update `Cargo.toml` version.
2. Update `Cargo.lock` if needed.
3. Add a dated release section to `CHANGELOG.md`.

## Verification

1. Run `make verify-release`.
2. Confirm both fixture suites still pass:
   - `tests/fixtures/blueprint_app`
   - `tests/fixtures/plugin_consumer_app`

## Tagging

1. Commit the release changes.
2. Create a Git tag using the format `v<version>`.
   Example: `v0.1.1`
3. Push the branch and tag.

## Publish

1. Run `cargo package --allow-dirty` one final time if anything changed after verification.
2. Run `cargo publish` manually when the package is ready.
3. Create the GitHub release notes from the tagged changelog entry.

## After Publish

1. Restore `CHANGELOG.md` with a fresh `Unreleased` section if needed.
2. Start the next iteration from the new released version line.
