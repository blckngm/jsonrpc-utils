set -ex

cargo clippy --all --no-default-features -- -D warnings
cargo clippy --all --no-default-features --features=client -- -D warnings
cargo clippy --all --features=client -- -D warnings
cargo clippy --all --features=blocking-client -- -D warnings
cargo clippy --all --all-targets --all-features -- -D warnings
cargo +nightly test --all --all-features --all-targets
