use libc;
use std::fs::File;
use std::os::unix::io::AsRawFd;

/// Do... unspeakable things.
///
/// Waits for a filedescriptor to yield a value because the default API doesn't
/// give us this option. Calls `poll(3)` on the file descriptor opened with an
/// infinite timeout and returns once data is available.
pub fn wait_for_data(fd: &mut libc::pollfd) {
    unsafe {
        if libc::poll(fd as *mut libc::pollfd, 1, -1) > 0 {
            if (*fd).revents & libc::POLLIN != 0 {
                return;
            }
        }
        unreachable!();
    }
}

/// Do... more unspeakable things.
///
/// Setup `pollfd` structure to use with the above waiting routine.
pub fn setup_pollfd(fd: &File) -> libc::pollfd {
    libc::pollfd {
        fd: fd.as_raw_fd(),
        events: libc::POLLIN,
        revents: 0
    }
}

