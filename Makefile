.PHONY: all help build release test test-fixtures fmt fmt-check clippy audit md-lint hooks check clean

all: check ## Default: run the local check suite

help: ## Show this help
	@grep -E '^[a-zA-Z_-]+:.*?## ' $(MAKEFILE_LIST) | awk 'BEGIN {FS = ":.*?## "}; {printf "  %-14s %s\n", $$1, $$2}'

build: ## Debug build
	cargo build

release: ## Release build (what the herdr manifest runs)
	cargo build --release

test: ## Unit and parser tests, no toolchains beyond Rust
	cargo test

test-fixtures: ## Run the real fixture projects too (uses whatever toolchains are installed)
	HERDR_TESTRUN_FIXTURES=1 cargo test --test fixtures -- --nocapture

fmt: ## Format sources
	cargo fmt --all

fmt-check: ## Fail if sources are not formatted
	cargo fmt --all --check

clippy: ## Lint, warnings are errors
	cargo clippy --all-targets -- -D warnings

audit: ## Check dependencies against the RustSec advisory database (needs cargo-audit)
	cargo audit

md-lint: ## Lint markdown
	markdownlint-cli2

hooks: ## Install pre-commit hooks
	pre-commit install

check: fmt-check clippy test audit md-lint ## Local suite, same as CI

clean: ## Remove build output
	cargo clean
