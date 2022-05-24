use std::fs::File;
use std::io::{self, BufReader, ErrorKind, Read, Write};
use std::net::TcpStream;
use std::os::unix::io::AsRawFd;
use std::os::unix::net::UnixStream;
use std::sync::Arc;

use nix::poll::{poll, PollFd, PollFlags};

use rustls::ServerConnection;

use anyhow::Context;

const BUFFER_SIZE: usize = 4096;

pub(crate) fn handler(
    http_trigger_socket: File,
    cert: File,
    key: File,
    mut stream: TcpStream,
) -> i32 {
    let (mut socket, far_socket) = UnixStream::pair().unwrap();

    let config = make_config(cert, key);
    let mut tls_conn = rustls::ServerConnection::new(config).unwrap();

    super::http_handler(&http_trigger_socket, far_socket);

    stream.set_nonblocking(true).unwrap();
    socket.set_nonblocking(true).unwrap();

    let mut to_poll = [
        PollFd::new(stream.as_raw_fd(), PollFlags::POLLIN),
        PollFd::new(socket.as_raw_fd(), PollFlags::POLLIN),
    ];

    loop {
        println!("starting polling");
        poll(&mut to_poll, -1).unwrap();

        if let Some(events) = to_poll[0].revents() {
            if events.contains(PollFlags::POLLIN) {
                handle_encrypted_data(&mut tls_conn, &mut stream, &mut socket).unwrap();
            }
        }

        if let Some(events) = to_poll[1].revents() {
            if events.contains(PollFlags::POLLIN) {
                handle_new_data(&mut tls_conn, &mut socket, &mut stream).unwrap();
            }

            if events.contains(PollFlags::POLLHUP) {
                println!("response writer hung up, exiting");
                break;
            }
        }
    }

    tls_conn.send_close_notify();
    tls_conn.write_tls(&mut stream).unwrap();

    exitcode::OK
}

fn handle_encrypted_data(
    tls_conn: &mut ServerConnection,
    stream: &mut (impl Read + Write),
    socket: &mut impl Write,
) -> anyhow::Result<()> {
    println!("handling newly received encrypted data");

    loop {
        let read = match tls_conn.read_tls(stream) {
            Err(e) => {
                if e.kind() == ErrorKind::WouldBlock {
                    0
                } else {
                    return Err(e).context("io error reading from stream");
                }
            }
            Ok(n) => n,
        };

        if read == 0 {
            return Ok(());
        }

        let process_result = tls_conn.process_new_packets();
        let write_tls_result = tls_conn.write_tls(stream);

        let io_state = process_result.context("tls processing failure")?;
        write_tls_result.context("tls write failure")?;

        if io_state.plaintext_bytes_to_read() > 0 {
            let mut reader = tls_conn
                .reader()
                .take(io_state.plaintext_bytes_to_read() as u64);

            std::io::copy(&mut reader, socket)?;
        }
    }
}

fn handle_new_data(
    tls_conn: &mut ServerConnection,
    socket: &mut impl Read,
    stream: &mut impl Write,
) -> anyhow::Result<()> {
    println!("handling new data to encrypt");

    let mut buf = [0_u8; BUFFER_SIZE];
    loop {
        let read = non_blocking_read(socket, &mut buf)?;
        if read == 0 {
            return Ok(());
        }

        tls_conn.writer().write_all(&buf[0..read])?;
        tls_conn.write_tls(stream)?;
    }
}

fn non_blocking_read(reader: &mut impl io::Read, buf: &mut [u8]) -> io::Result<usize> {
    match reader.read(buf) {
        Err(e) => {
            if e.kind() == ErrorKind::WouldBlock {
                Ok(0)
            } else {
                Err(e)
            }
        }
        Ok(n) => Ok(n),
    }
}

fn make_config(cert: File, key: File) -> Arc<rustls::ServerConfig> {
    let certs = load_certs(cert);
    let privkey = load_private_key(key);

    let config = rustls::ServerConfig::builder()
        .with_safe_default_cipher_suites()
        .with_safe_default_kx_groups()
        .with_safe_default_protocol_versions()
        .expect("inconsistent cipher-suites/versions specified")
        .with_no_client_auth()
        .with_single_cert(certs, privkey)
        .expect("bad certificates/private key");

    Arc::new(config)
}

fn load_certs(certfile: File) -> Vec<rustls::Certificate> {
    let mut reader = BufReader::new(certfile);

    rustls_pemfile::certs(&mut reader)
        .unwrap()
        .iter()
        .map(|v| rustls::Certificate(v.clone()))
        .collect()
}

fn load_private_key(keyfile: File) -> rustls::PrivateKey {
    let mut reader = BufReader::new(keyfile);

    loop {
        match rustls_pemfile::read_one(&mut reader).expect("cannot parse private key .pem file") {
            Some(rustls_pemfile::Item::RSAKey(key)) => return rustls::PrivateKey(key),
            Some(rustls_pemfile::Item::PKCS8Key(key)) => return rustls::PrivateKey(key),
            Some(rustls_pemfile::Item::ECKey(key)) => return rustls::PrivateKey(key),
            None => break,
            _ => {}
        }
    }

    panic!("no keys found (encrypted keys not supported)");
}
