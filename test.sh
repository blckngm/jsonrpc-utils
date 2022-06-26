set -ex

cargo clippy --all --no-default-features
cargo clippy --all --no-default-features --features=client
cargo clippy --all --features=client
cargo clippy --all --features=blocking-client
cargo clippy --all --all-targets --all-features
cargo +nightly test --all --all-features --all-targets
