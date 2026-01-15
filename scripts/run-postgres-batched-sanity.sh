#!/bin/bash
./scripts/stop-docker.sh all
cargo build --release
cargo run --release --bin coordinator -- -c config.sanity-postgresql-batched.toml
