ROOT_DIR := $(dir $(realpath $(lastword $(MAKEFILE_LIST))))

.PHONY: setup mock-data infra up down clean dev-api dev-frontend test test-cov

# Install dependencies (first time setup)
setup:
	cd $(ROOT_DIR)backend && uv sync
	cd $(ROOT_DIR)frontend && npm install

# Generate mock data
mock-data:
	python3 $(ROOT_DIR)scripts/generate_all_mock_data.py

# Start infrastructure only (Neo4j + data import)
infra: mock-data
	docker compose up -d neo4j neo4j-init

# Stop infrastructure only
infra_down:
	docker compose stop neo4j

# Start infrastructure without re-importing data
infra_up:
	docker compose start neo4j

# Start all services
up:
	docker compose up -d

# Stop all services
down:
	docker compose down

# Clean up (remove containers, volumes, generated data)
clean:
	docker compose down -v
	rm -f scripts/output/*.csv scripts/output/*.cypher

# Dev mode: start API only (Neo4j must be running)
dev-api:
	cd $(ROOT_DIR)backend && uv run uvicorn app.main:app --reload --host 0.0.0.0 --port 8000

# Dev mode: start frontend only
dev-frontend:
	cd $(ROOT_DIR)frontend && npm run dev

# Run all tests
test:
	cd $(ROOT_DIR)backend && uv run python -m pytest tests/ -v -p no:asyncio
	cd $(ROOT_DIR)frontend && npm test

# Run backend tests with coverage
test-cov:
	cd $(ROOT_DIR)backend && uv run python -m pytest tests/ -v -p no:asyncio --cov=app --cov-report=term-missing

