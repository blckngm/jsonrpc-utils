export CARGO_PROFILE_RELEASE_LTO=thin

cargo build --all-features --release --examples

taskset -c 0,2,4,6 cargo run --all-features --release --example old-server &
taskset -c 0,2,4,6 cargo run --all-features --release --example server-macros &

sleep 2

function bench() {
    curl -v -H 'content-type: application/json' -d '{"jsonrpc":"2.0","id":1,"method":"@ping"}' 127.0.0.1:$1/rpc
    # Warm up
    taskset -c 8,10,12 rewrk -t3 -c150 -d 1s  --host http://127.0.0.1:$1/rpc -m post --header 'content-type: application/json' -b '{"jsonrpc":"2.0","id":1,"method":"@ping"}' > /dev/null
    taskset -c 8,10,12 rewrk -t3 -c150 -d 10s  --host http://127.0.0.1:$1/rpc -m post --header 'content-type: application/json' -b '{"jsonrpc":"2.0","id":1,"method":"@ping"}'
}

echo new

bench 3000

echo old

bench 3001
