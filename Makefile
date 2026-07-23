## SRE Inspection Graph — top-level entry for the active stack (Rust + WASM + Tauri).
##
## Legacy Python targets are prefixed `ref-` (reference/ is a read-only oracle).
## Active stack: engine (Rust workspace) + modules (WASM, wasm32-wasip2) + desktop (Tauri).
##
## Quick start:
##   make help                 list all targets
##   make test-all             full gate (engine + desktop)
##   make check-all            clippy gate (engine + desktop + modules)
##   make build-all            release build everything
##   make desktop-dev          run the Tauri app (needs `make modules-build` once)

ROOT_DIR := $(dir $(realpath $(lastword $(MAKEFILE_LIST))))
ENGINE   := $(ROOT_DIR)engine
MODULES  := $(ROOT_DIR)modules
DESKTOP  := $(ROOT_DIR)desktop
## Default SQLite path (Linux app data dir); override with `make ... DB=/path`.
DEFAULT_DB := $(HOME)/.local/share/io.sregraph.desktop/sre-graph.sqlite

.DEFAULT_GOAL := help

.PHONY: help test-all check-all build-all \
        engine-build engine-check engine-test engine-clippy engine-fmt \
        engine-cli-tick engine-cli-tick-loop engine-dump-topology engine-inspect-views \
        modules-build modules-build-debug modules-check modules-clippy modules-build-one \
        desktop-setup desktop-dev desktop-build desktop-test desktop-web \
        ref-setup ref-dev-api ref-test ref-test-cov ref-infra ref-up ref-down ref-clean \
        clean clean-rust

# ---------------------------------------------------------------
#  Help
# ---------------------------------------------------------------

help: ## Show this help.
	@printf '\nAvailable targets (active stack unless prefixed ref-):\n\n'
	@awk 'BEGIN {FS = ":.*?## "} /^[a-zA-Z_-]+:.*?## / {printf "  \033[36m%-22s\033[0m %s\n", $$1, $$2}' $(MAKEFILE_LIST)
	@printf '\nOverride vars: MOD=modules/connectors/k8s  DB=/path/to.sqlite\n\n'

# ---------------------------------------------------------------
#  Active stack — full gate / aggregates
# ---------------------------------------------------------------

test-all: engine-test desktop-test ## Run the full test gate (engine Rust + desktop vitest).

check-all: engine-clippy desktop-check modules-clippy ## Run clippy -D warnings across all three workspaces.

build-all: engine-build modules-build desktop-build ## Release-build engine + WASM modules + desktop.

# ---------------------------------------------------------------
#  Engine (Rust workspace)
# ---------------------------------------------------------------

engine-build: ## cargo build --release (engine workspace).
	cd $(ENGINE) && cargo build --release

engine-check: ## cargo check --all-targets (fast type-check).
	cd $(ENGINE) && cargo check --all-targets

engine-test: ## cargo test --workspace.
	cd $(ENGINE) && cargo test --workspace

engine-clippy: ## cargo clippy --workspace --all-targets -- -D warnings.
	cd $(ENGINE) && cargo clippy --workspace --all-targets -- -D warnings

engine-fmt: ## cargo fmt --all -- --check.
	cd $(ENGINE) && cargo fmt --all -- --check

engine-cli-tick: ## Headless one-shot sync_all (loads manifest + runs all connectors).
	cd $(ENGINE) && cargo run -p engine-cli --release -- tick

engine-cli-tick-loop: ## Headless loop sync (Ctrl-C to stop). Override INTERVAL=30.
	cd $(ENGINE) && cargo run -p engine-cli --release -- tick --loop --interval=$(INTERVAL)

engine-dump-topology: ## GUI-less dump of materialized topology as GraphResponse. DB=/path.sqlite
	cd $(ENGINE) && cargo run -p engine-storage --release --example dump_topology -- $(DB)

engine-inspect-views: ## GUI-less verify all 6 inspection views against live SQLite. DB=/path.sqlite
	cd $(ENGINE) && cargo run -p engine-storage --release --example inspect_views -- $(DB)

engine-archive: ## Snapshot current SQLite latest facts into a Parquet archive. DB=... ARCHIVE=dir
	cd $(ENGINE) && cargo run -p engine-storage --release --example archive_facts -- $(DB) $(ARCHIVE)

# ---------------------------------------------------------------
#  WASM modules (separate workspace, wasm32-wasip2)
# ---------------------------------------------------------------
# `cargo wasi-build` etc. are aliases defined in modules/.cargo/config.toml
# (host-target `cargo test` still works — default target is intentionally unset).

modules-build: ## Build all WASM modules (release, wasm32-wasip2).
	cd $(MODULES) && cargo wasi-build

modules-build-debug: ## Debug build of all WASM modules (faster iteration).
	cd $(MODULES) && cargo wasi-build-debug

modules-check: ## Type-check WASM modules (no artifact, fastest).
	cd $(MODULES) && cargo wasi-check

modules-clippy: ## clippy on WASM module code paths (--target wasm32-wasip2 -D warnings).
	cd $(MODULES) && cargo wasi-clippy

modules-build-one: ## Build one module: make modules-build-one MOD=modules/connectors/k8s
	cd $(ROOT_DIR)$(MOD) && cargo wasi-build

# ---------------------------------------------------------------
#  Desktop (Tauri 2.x — React 18 + AntD + Cytoscape)
# ---------------------------------------------------------------

desktop-setup: ## npm install desktop deps.
	cd $(DESKTOP) && npm install

desktop-dev: ## Run the full Tauri app (webview + Rust backend).
	cd $(DESKTOP) && npm run tauri dev

desktop-web: ## Vite-only frontend (no Rust backend; for UI iteration).
	cd $(DESKTOP) && npm run dev

desktop-build: ## Production build (.AppImage / .msi / .app).
	cd $(DESKTOP) && npm run tauri build

desktop-test: ## desktop vitest.
	cd $(DESKTOP) && npm test

desktop-check: ## desktop Rust backend check + frontend tsc.
	cd $(DESKTOP)/src-tauri && cargo check --all-targets
	cd $(DESKTOP) && npx tsc --noEmit

# ---------------------------------------------------------------
#  Legacy Python reference (read-only oracle — DO NOT MODIFY)
# ---------------------------------------------------------------
# Run the old FastAPI stack locally to compare Rust behavior against the spec.
# Never deploy reference/; it is behavior reference only.

ref-setup: ## uv sync the reference Python env.
	cd $(ROOT_DIR)reference && uv sync

ref-dev-api: ## Run reference FastAPI oracle on 8000 (desktop kubectl proxy owns 8001).
	cd $(ROOT_DIR)reference && uv run uvicorn app.main:app --reload --host 0.0.0.0 --port 8000

ref-test: ## Run reference backend tests (472). Must use -p no:asyncio.
	cd $(ROOT_DIR)reference && uv run python -m pytest tests/ -v -p no:asyncio

ref-test-cov: ## Reference backend tests with coverage.
	cd $(ROOT_DIR)reference && uv run python -m pytest tests/ -v -p no:asyncio --cov=app --cov-report=term-missing

ref-infra: ## Start reference Neo4j (docker compose).
	cd $(ROOT_DIR) && docker compose up -d neo4j neo4j-init

ref-up: ## Start all reference docker compose services.
	cd $(ROOT_DIR) && docker compose up -d

ref-down: ## Stop reference docker compose.
	cd $(ROOT_DIR) && docker compose down

# ---------------------------------------------------------------
#  Clean
# ---------------------------------------------------------------

clean: ## Light clean — reference docker volumes + generated CSV/cypher.
	cd $(ROOT_DIR) && docker compose down -v 2>/dev/null || true
	rm -f $(ROOT_DIR)scripts/output/*.csv $(ROOT_DIR)scripts/output/*.cypher

clean-rust: ## Heavy clean — wipe engine + modules target dirs (expensive rebuild).
	cd $(ENGINE) && cargo clean
	cd $(MODULES) && cargo clean
