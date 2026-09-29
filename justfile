set shell := ["bash", "-euo", "pipefail", "-c"]

default: check test clippy

check:
    cargo check

test:
    cargo test

clippy:
    cargo clippy --all-targets -- -D warnings

build:
    cargo build --release

run *args:
    cargo run -- {{args}}

fmt:
    cargo fmt
