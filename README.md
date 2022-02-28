# clone-shim

## Running the examples

### examples/basic

The basic example instructs the shim to spawn two processes, each of which writes "hello from main{1,2}!" to stdout.

To run this example:

    cargo build --example basic
    cargo run -- -s examples/basic/spec.json target/debug/examples/basic
