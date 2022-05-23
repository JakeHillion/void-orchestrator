use std::fs::File;
use std::net::TcpStream;

pub(crate) fn handler(
    http_trigger_socket: File,
    _cert: File,
    _key: File,
    stream: TcpStream,
) -> i32 {
    super::http_handler(&http_trigger_socket, stream);
    exitcode::OK
}
