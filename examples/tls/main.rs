mod http;

use std::fs::File;
use std::net::{TcpListener, TcpStream};

fn main() {
    let mut args = std::env::args();

    let _bin = args.next();
    let entrypoint = args.next();

    match entrypoint {
        Some(s) => match s.as_str() {
            "connection_listener" => connection_listener_entrypoint(),
            "http_handler" => http_handler_entrypoint(),

            _ => unimplemented!(),
        },
        None => unimplemented!(),
    }
}

fn connection_listener_entrypoint() {
    // imports
    use std::os::unix::io::{FromRawFd, RawFd};

    // argument parsing
    let mut args = std::env::args();

    let _bin = args.next();
    let _entrypoint = args.next();

    let http_handler_trigger = args.next();
    let http_handler_trigger: RawFd = http_handler_trigger
        .expect("request handler required")
        .parse()
        .expect("tcp listener should be a file descriptor");
    let http_handler_trigger = unsafe { File::from_raw_fd(http_handler_trigger) };

    let tcp_listener = args.next();
    let tcp_listener: RawFd = tcp_listener
        .expect("tcp listener required")
        .parse()
        .expect("tcp listener should be a file descriptor");
    let tcp_listener = unsafe { TcpListener::from_raw_fd(tcp_listener) };

    // actual function body
    fn connection_listener(http_handler_trigger: File, tcp_listener: TcpListener) -> i32 {
        println!("connection_listener entered");

        // handle incoming connections
        for stream in tcp_listener.incoming() {
            let stream = match stream {
                Ok(s) => s,
                Err(e) => {
                    println!("connection listener: error: {}", e);
                    return 1;
                }
            };

            println!("received a new connection");
            http_handler(&http_handler_trigger, stream);
        }

        exitcode::OK
    }

    // run function
    std::process::exit(connection_listener(http_handler_trigger, tcp_listener));
}

fn http_handler(trigger_socket: &File, stream: TcpStream) {
    // imports
    use nix::sys::socket::{sendmsg, ControlMessage, MsgFlags};
    use std::os::unix::io::AsRawFd;

    // send file descriptor(s)
    let sockfd = trigger_socket.as_raw_fd();
    let fds = [stream.as_raw_fd()];

    sendmsg::<()>(
        sockfd,
        &[],
        &[ControlMessage::ScmRights(&fds)],
        MsgFlags::empty(),
        None,
    )
    .unwrap();
}

fn http_handler_entrypoint() {
    // imports
    use std::os::unix::io::{FromRawFd, RawFd};

    // argument parsing
    let mut args = std::env::args();

    let _bin = args.next();
    let _entrypoint = args.next();

    let stream = args.next();
    let stream: RawFd = stream
        .expect("request stream required")
        .parse()
        .expect("request stream should be a file descriptor");
    let stream = unsafe { TcpStream::from_raw_fd(stream) };

    std::process::exit(http::handler(stream));
}
