clean:
    rm -fRd target

lint:
    set -e
    cargo fmt -- --check
    cargo clippy --all-features --all-targets -- -D warnings

test:
    just lint
    cargo test -- --nocapture

