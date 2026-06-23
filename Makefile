ROOT_DIR := $(dir $(realpath $(lastword $(MAKEFILE_LIST))))

.PHONY: setup mock-data infra infra_down infra_up up down clean \
        ref-setup ref-dev-api ref-test ref-test-cov \
        dev-api dev-api-kill dev-frontend test test-cov \
        engine-build engine-check engine-test engine-clippy engine-fmt \
        desktop-dev desktop-build desktop-test \
        modules-build specs-generate-tauri-types contract-test

# ---------------------------------------------------------------
# Legacy Python (now in reference/) — kept for behavior oracle only
# ---------------------------------------------------------------

# Install dependencies (first time setup,reference + frontend)
setup: ref-setup
	cd $(ROOT_DIR)frontend && npm install

ref-setup:
	cd $(ROOT_DIR)reference && uv sync

# Run reference Python API as oracle on port 8001 (not 8000)
ref-dev-api:
	cd $(ROOT_DIR)reference && uv run uvicorn app.main:app --reload --host 0.0.0.0 --port 8001

# Run reference Python tests (472)
ref-test:
	cd $(ROOT_DIR)reference && uv run python -m pytest tests/ -v -p no:asyncio

ref-test-cov:
	cd $(ROOT_DIR)reference && uv run python -m pytest tests/ -v -p no:asyncio --cov=app --cov-report=term-missing

# Generate mock data
mock-data:
	python3 $(ROOT_DIR)scripts/generate_all_mock_data.py

# Start infrastructure only (Neo4j + data import)
infra: mock-data
	docker compose up -d neo4j neo4j-init

infra_down:
	docker compose stop neo4j

infra_up:
	docker compose start neo4j

up:
	docker compose up -d

down:
	docker compose down

clean:
	docker compose down -v
	rm -f scripts/output/*.csv scripts/output/*.cypher

# ---------------------------------------------------------------
# Backward-compat aliases (Phase 1 transition,Phase 5 deleted)
# ---------------------------------------------------------------

dev-api-kill:
	@lsof -ti:8000 | xargs kill -9 2>/dev/null; echo "port 8000 freed"

dev-api: dev-api-kill ref-dev-api
dev-frontend:
	cd $(ROOT_DIR)frontend && npm run dev

test: ref-test
	cd $(ROOT_DIR)frontend && npm test

test-cov: ref-test-cov

# ---------------------------------------------------------------
# Rust engine (Phase 1+) — primary build path
# ---------------------------------------------------------------

engine-build:
	cd $(ROOT_DIR)engine && cargo build --release

engine-check:
	cd $(ROOT_DIR)engine && cargo check --all-targets

engine-test:
	cd $(ROOT_DIR)engine && cargo test

engine-clippy:
	cd $(ROOT_DIR)engine && cargo clippy --all-targets -- -D warnings

engine-fmt:
	cd $(ROOT_DIR)engine && cargo fmt --all -- --check

# ---------------------------------------------------------------
# Tauri desktop (Phase 1 skeleton,Phase 2 wired)
# ---------------------------------------------------------------

desktop-dev:
	cd $(ROOT_DIR)desktop && npm run tauri dev

desktop-build:
	cd $(ROOT_DIR)desktop && npm run tauri build

desktop-test:
	cd $(ROOT_DIR)desktop && npm test

# ---------------------------------------------------------------
# WASM modules
# ---------------------------------------------------------------

modules-build:
	cd $(ROOT_DIR)modules && cargo build --release --target wasm32-wasip2

specs-generate-tauri-types:
	cd $(ROOT_DIR)desktop/src-tauri && cargo run --bin generate-types

contract-test:
	cd $(ROOT_DIR)tests/contract && cargo test
