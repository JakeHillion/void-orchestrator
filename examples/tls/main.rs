use std::net::TcpListener;

fn main() {
    let mut args = std::env::args();

    let _bin = args.next();
    let entrypoint = args.next();

    match entrypoint {
        Some(s) => match s.as_str() {
            "connection_listener" => connection_listener_entrypoint(),

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

    let tcp_listener = args.next();
    let tcp_listener: RawFd = tcp_listener
        .expect("tcp listener required")
        .parse()
        .expect("tcp listener should be a file descriptor");
    let tcp_listener = unsafe { TcpListener::from_raw_fd(tcp_listener) };

    // actual function body
    fn connection_listener(tcp_listener: TcpListener) -> i32 {
        println!("connection_listener entered");

        // handle incoming connections
        for stream in tcp_listener.incoming() {
            let _stream = match stream {
                Ok(s) => s,
                Err(e) => {
                    println!("connection listener: error: {}", e);
                    return 1;
                }
            };

            println!("received a new connection");
        }

        exitcode::OK
    }

    // run function
    std::process::exit(connection_listener(tcp_listener));
}
