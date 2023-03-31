set -ex

cargo clippy --all -- -D warnings
cargo clippy --all --features=client -- -D warnings
cargo clippy --all --features=blocking-client -- -D warnings
cargo clippy --all --features=server -- -D warnings
cargo clippy --all --features=axum -- -D warnings
cargo clippy --all --all-targets --all-features -- -D warnings
cargo +nightly test --all --all-features --all-targets
