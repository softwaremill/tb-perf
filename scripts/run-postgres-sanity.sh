#!/bin/bash
cargo build --release
cargo run --release --bin coordinator -- -c config.sanity-postgresql.toml
