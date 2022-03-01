# clone-shim

## Running the examples

### examples/basic

The basic example instructs the shim to spawn two processes, each of which writes "hello from main{1,2}!" to stdout.

To run this example:

    cargo build --example basic
    cargo run -- -s examples/basic/spec.json target/debug/examples/basic

### examples/pipes

The pipes example shows some of the power of the shim by using pipes. The process "pipe_sender" sends two messages down a pipe that it's given by the shim. These two messages each spawn a completely isolated process, "pipe_receiver", that receives that message.

To run this example:

    cargo build --example pipes
    cargo run -- -s examples/pipes/spec.json target/debug/examples/pipes
