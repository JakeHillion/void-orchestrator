# clone-shim

## Running the examples

### examples/basic

The basic example instructs the shim to spawn two processes, each of which writes "hello from main{1,2}!" to stdout.

To run this example:

    cargo build
    cargo build --example basic
    target/debug/clone-shim -s examples/basic/spec.json target/debug/examples/basic

### examples/pipes

The pipes example shows some of the power of the shim by using pipes. The process "pipe_sender" sends two messages down a pipe that it's given by the shim. These two messages each spawn a completely isolated process, "pipe_receiver", that receives that message.

To run this example:

    cargo build
    cargo build --example pipes
    target/debug/clone-shim -s examples/pipes/spec.json target/debug/examples/pipes

## Debugging the shim

The shim can be debugged as with most processes, but it is exceptionally forky. Breaking before a clone in `rust-gdb` then running `set follow-fork-mode child` is often necessary. The best approach is to go in with a plan of attack.

## Debugging the child

Debugging the child processes is vastly more difficult than in other more Linux-like containerisation solutions.

The `--debug` flag on the shim attempts to stop application spawned processes as soon as they are voided. This gives you a chance to attach with a debugger.

The debugger must be run from the ambient namespace and not within the void, as none of the prerequisites will exist within the void.

Good luck!
