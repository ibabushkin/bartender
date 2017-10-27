use libc;

use std::fs::File;
use std::io::{BufRead, BufReader};
use std::os::unix::io::AsRawFd;

/// Set up a file to use with `poll`.
pub fn setup_pollfd(fd: &File) -> libc::pollfd {
    libc::pollfd {
        fd: fd.as_raw_fd(),
        events: libc::POLLIN,
        revents: 0,
    }
}

/// Poll a set of filedescriptors perviously wrapped.
pub fn poll(fds: &mut [libc::pollfd]) -> bool {
    let poll_res = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as u64, -1) };
    poll_res > 0
}

/// A wrapped `BufReader` only yielding complete lines, annotated with
/// an index.
pub struct FileBuffer(pub BufReader<File>, pub usize);

/// Message type sent through our channels.
pub type Message = Vec<(usize, String)>;

/// Fill some buffers from a set of previously `poll`ed filedsecriptors.
pub fn get_lines(fds: &[libc::pollfd], buffers: &mut [FileBuffer]) -> Message {
    let fd_len = fds.len();
    let mut res = Vec::with_capacity(fd_len);
    for (fd, &mut FileBuffer(ref mut reader, ref id)) in fds.iter().zip(buffers) {
        if fd.fd != reader.get_ref().as_raw_fd() {
            panic!("error: mismatched FileBuffer. this is a bug - please file an issue.");
        }

        if fd.revents & libc::POLLIN != 0 {
            let mut value = String::new();
            if reader.read_line(&mut value).is_ok() {
                if value.len() > 0 && value.as_bytes()[value.len() - 1] == 0xA {
                    value.pop();
                }

                res.push((*id, value));
            }
        }
    }

    res
}
