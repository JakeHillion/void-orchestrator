use std::fs::OpenOptions;
use std::io::{self, ErrorKind, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;

pub(super) fn handler(mut stream: UnixStream) -> i32 {
    println!("entered http handler");

    let mut buf = Vec::new();
    let mut buf_len = 0;

    loop {
        buf.resize_with(buf_len + 4096, Default::default);

        let read_bytes = stream.read(&mut buf[buf_len..]).unwrap();
        buf_len += read_bytes;

        if read_bytes == 0 {
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

        if let Some(filename) = filename {
            if try_serve_file(&mut stream, filename).unwrap() {
                return exitcode::OK;
            }
        }

        let status_line = "HTTP/1.1 404 NOT FOUND";
        let contents = "file not found\n";

        let response = format!(
            "{}\r\nContent-Length: {}\r\n\r\n{}",
            status_line,
            contents.len(),
            contents
        );

        stream.write_all(response.as_bytes()).unwrap();
        break;
    }

    exitcode::OK
}

fn try_serve_file(stream: &mut impl io::Write, filename: &str) -> io::Result<bool> {
    let mut fd = match OpenOptions::new()
        .read(true)
        .open(PathBuf::from("/var/www/html/").join(filename.strip_prefix('/').unwrap_or(filename)))
    {
        Ok(fd) => fd,
        Err(e) => {
            if e.kind() == ErrorKind::NotFound {
                return Ok(false);
            }
            return Err(e);
        }
    };

    let status_line = "HTTP/1.1 200 OK";

    let response_header = format!(
        "{}\r\nContent-Length: {}\r\n\r\n",
        status_line,
        fd.metadata()?.len(),
    );

    stream.write_all(response_header.as_bytes())?;
    io::copy(&mut fd, stream)?;

    Ok(true)
}
