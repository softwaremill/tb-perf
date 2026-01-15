#!/bin/bash
./scripts/stop-docker.sh all
cargo build --release

./scripts/tigerbeetle-local.sh wipe
./scripts/tigerbeetle-local.sh start
docker compose -f docker/docker-compose.tigerbeetle.yml -p tbperf up -d otel-collector prometheus grafana

./target/release/coordinator -c config.sanity-tigerbeetle.toml --no-docker