mod http;
mod listener;
mod tls;

use std::fs::File;
use std::net::{TcpListener, TcpStream};
use std::os::unix::net::UnixStream;

fn main() {
    match std::env::args().next() {
        Some(s) => match s.as_str() {
            "connection_listener" => connection_listener_entrypoint(),
            "tls_handler" => tls_handler_entrypoint(),
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

    let _entrypoint = args.next();

    let tls_handler_trigger = args.next();
    let tls_handler_trigger: RawFd = tls_handler_trigger
        .expect("tls handler trigger required")
        .parse()
        .expect("tls handler trigger should be a file descriptor");
    let tls_handler_trigger = unsafe { File::from_raw_fd(tls_handler_trigger) };

    let tcp_listener = args.next();
    let tcp_listener: RawFd = tcp_listener
        .expect("tcp listener required")
        .parse()
        .expect("tcp listener should be a file descriptor");
    let tcp_listener = unsafe { TcpListener::from_raw_fd(tcp_listener) };

    // run function
    std::process::exit(listener::handler(tls_handler_trigger, tcp_listener));
}

fn tls_handler(trigger_socket: &File, stream: TcpStream) {
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

fn tls_handler_entrypoint() {
    // imports
    use std::os::unix::io::{FromRawFd, RawFd};

    // argument parsing
    let mut args = std::env::args();

    let _entrypoint = args.next();

    let http_handler_trigger = args.next();
    let http_handler_trigger: RawFd = http_handler_trigger
        .expect("http handler trigger required")
        .parse()
        .expect("http handler trigger should be a file descriptor");
    let http_handler_trigger = unsafe { File::from_raw_fd(http_handler_trigger) };

    let tls_cert_file = args.next();
    let tls_cert_file: RawFd = tls_cert_file
        .expect("tls cert file required")
        .parse()
        .expect("tls cert file should be a file descriptor");
    let tls_cert_file = unsafe { File::from_raw_fd(tls_cert_file) };

    let tls_key_file = args.next();
    let tls_key_file: RawFd = tls_key_file
        .expect("tls key file required")
        .parse()
        .expect("tls key file should be a file descriptor");
    let tls_key_file = unsafe { File::from_raw_fd(tls_key_file) };

    let stream = args.next();
    let stream: RawFd = stream
        .expect("request stream required")
        .parse()
        .expect("request stream should be a file descriptor");
    let stream = unsafe { TcpStream::from_raw_fd(stream) };

    std::process::exit(tls::handler(
        http_handler_trigger,
        tls_cert_file,
        tls_key_file,
        stream,
    ));
}

fn http_handler(trigger_socket: &File, stream: UnixStream) {
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

    let _entrypoint = args.next();

    let stream = args.next();
    let stream: RawFd = stream
        .expect("request stream required")
        .parse()
        .expect("request stream should be a file descriptor");
    let stream = unsafe { UnixStream::from_raw_fd(stream) };

    std::process::exit(http::handler(stream));
}
