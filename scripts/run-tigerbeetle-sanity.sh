#!/bin/bash
cargo build --release

./scripts/tigerbeetle-local.sh start
docker compose -f docker/docker-compose.tigerbeetle.yml -p tbperf up -d otel-collector prometheus grafana

./target/release/coordinator -c config.sanity-tigerbeetle.toml --no-docker

./scripts/tigerbeetle-local.sh wipe
./scripts/stop-docker.sh tigerbeetle