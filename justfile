default:
    just --list

test:
    cargo test

dev:
    cargo run

lint:
    cargo clippy

ci:
    cargo fmt --check
    cargo clippy --all-targets -- -D warnings
    cargo test

fmt:
    cargo fmt
