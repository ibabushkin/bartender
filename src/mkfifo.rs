use libc;

use std::ffi::CString;
use std::fs::{File, OpenOptions};
use std::os::unix::fs::FileTypeExt;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

pub fn open_fifo(path: &Path) -> Option<File> {
    let mut options = OpenOptions::new();
    options.read(true);
    options.write(true);

    // we open the file in read-write mode to prevent our poll()
    // hack from sending us `POLLHUP`s when no process is at the
    // other end of the pipe, so it blocks either way.
    match options.open(path) {
        Ok(f) => {
            match f.metadata().map(|m| m.file_type().is_fifo()) {
                Ok(true) => Some(f),
                _ => None, // anything decidedly non-fifo
            }
        }
        _ => {
            let path_cstr = CString::new(path.as_os_str().as_bytes()).unwrap();
            let path_ptr = path_cstr.as_ptr();
            let perms = libc::S_IRUSR | libc::S_IWUSR;
            let ret = unsafe { libc::mkfifo(path_ptr as *const i8, perms) };
            if ret != 0 { None } else { options.open(path).ok() }
        }
    }
}
