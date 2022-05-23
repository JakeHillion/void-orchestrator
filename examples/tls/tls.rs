use std::fs::File;
use std::io::{self, ErrorKind, Read, Write};
use std::net::TcpStream;
use std::os::unix::io::AsRawFd;
use std::os::unix::net::UnixStream;

use nix::poll::{poll, PollFd, PollFlags};

const BUFFER_SIZE: usize = 4096;

pub(crate) fn handler(
    http_trigger_socket: File,
    _cert: File,
    _key: File,
    mut stream: TcpStream,
) -> i32 {
    let (mut socket, far_socket) = UnixStream::pair().unwrap();

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
                handle_encrypted_data(&mut stream, &mut socket).unwrap();
            }
        }

        if let Some(events) = to_poll[1].revents() {
            if events.contains(PollFlags::POLLIN) {
                handle_new_data(&mut socket, &mut stream).unwrap();
            }

            if events.contains(PollFlags::POLLHUP) {
                println!("response writer hung up, exiting");
                break;
            }
        }
    }

    exitcode::OK
}

fn handle_encrypted_data(stream: &mut impl Read, socket: &mut impl Write) -> io::Result<()> {
    let mut buf = [0_u8; BUFFER_SIZE];

    loop {
        let read = non_blocking_read(stream, &mut buf)?;
        if read == 0 {
            return Ok(());
        }

        socket.write_all(&buf[0..read]).unwrap();
    }
}

fn handle_new_data(socket: &mut impl Read, stream: &mut impl Write) -> io::Result<()> {
    let mut buf = [0_u8; BUFFER_SIZE];

    loop {
        let read = non_blocking_read(socket, &mut buf)?;
        if read == 0 {
            return Ok(());
        }

        stream.write_all(&buf[0..read]).unwrap();
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
