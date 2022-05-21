use std::fs;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;

pub(super) fn handler(mut stream: TcpStream) -> i32 {
    println!("entered http handler");

    let mut buf = Vec::new();
    loop {
        let buf_len = buf.len();
        buf.resize_with(buf_len + 1024, Default::default);

        if stream.read(&mut buf[buf_len..]).unwrap() == 0 {
            break;
        }

        let mut headers = [httparse::EMPTY_HEADER; 64];
        let mut req = httparse::Request::new(&mut headers);
        let result = req.parse(&buf).unwrap();

        if result.is_partial() {
            continue;
        }

        let filename = if req.method != Some("GET") {
            None
        } else {
            req.path
        };

        let status_line = if filename.is_some() {
            "HTTP/1.1 200 OK"
        } else {
            "HTTP/1.1 404 NOT FOUND"
        };

        let contents = if let Some(filename) = filename {
            fs::read_to_string(
                PathBuf::from("/var/www/html/")
                    .join(filename.strip_prefix('/').unwrap_or(filename)),
            )
            .unwrap()
        } else {
            "content not found\n".to_string()
        };

        let response_header = format!(
            "{}\r\nContent-Length: {}\r\n\r\n",
            status_line,
            contents.len(),
        );

        stream.write_all(response_header.as_bytes()).unwrap();
        stream.write_all(contents.as_bytes()).unwrap();

        break;
    }

    exitcode::OK
}
