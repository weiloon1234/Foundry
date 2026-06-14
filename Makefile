.PHONY: fmt fmt-check test test-postgres fixture-check clippy package-check verify verify-release api-docs

fmt:
	cargo fmt

fmt-check:
	cargo fmt --check

test:
	cargo test --all-targets

test-postgres:
	cargo test --test database_acceptance

fixture-check:
	cargo test --test blueprint_fixture_acceptance
	cargo test --test plugin_fixture_acceptance

clippy:
	cargo clippy --all-targets -- -D warnings

package-check:
	cargo package --allow-dirty -p foundry-build
	cargo package --allow-dirty -p foundry-macros
	@tmp=$$(mktemp); \
	if cargo package --allow-dirty -p foundry >$$tmp 2>&1; then \
		cat $$tmp; \
		rm -f $$tmp; \
	elif grep -Eq 'no matching package named `(foundry-build|foundry-macros)` found' $$tmp; then \
		cat $$tmp; \
		echo "foundry root package verification needs foundry-build and foundry-macros in the target registry; publish/verify those support crates first, then rerun cargo package --allow-dirty -p foundry."; \
		rm -f $$tmp; \
	else \
		status=$$?; \
		cat $$tmp; \
		rm -f $$tmp; \
		exit $$status; \
	fi

verify: fmt-check test clippy fixture-check

verify-release: verify package-check

api-docs:
	cargo doc --no-deps
	cargo run --manifest-path tools/foundry-api-doc/Cargo.toml -- --output-dir docs/api
