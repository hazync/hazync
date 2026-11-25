//! Capture what the guest prints during an execution (OpenVM's print phantom writes to the host's stdout).
//! Used only for the journal-emitting execute pass; proving runs never print.
use std::{fs::File, io::{Read, Seek, SeekFrom, Write}};

pub fn stdout_of<T>(f: impl FnOnce() -> T) -> (T, String) {
    std::io::stdout().flush().ok();
    let mut tmp = tempfile();
    let r = redirected(&tmp, f);
    let mut s = String::new();
    tmp.seek(SeekFrom::Start(0)).unwrap();
    tmp.read_to_string(&mut s).unwrap();
    (r, s)
}

/// Run `f` with this process's stdout pointed at `to`, then point it back.
#[cfg(unix)]
fn redirected<T>(to: &File, f: impl FnOnce() -> T) -> T {
    use std::os::fd::AsRawFd;
    // SAFETY: plain fd juggling on this process's own stdout.
    let saved = unsafe { libc::dup(1) };
    assert!(saved >= 0, "dup(stdout)");
    unsafe { assert!(libc::dup2(to.as_raw_fd(), 1) >= 0, "dup2") };
    let r = f();
    std::io::stdout().flush().ok();
    unsafe {
        libc::dup2(saved, 1);
        libc::close(saved);
    }
    r
}

/// Windows: Rust's stdout asks for the process's standard output handle on every write, so swapping that handle
/// (SetStdHandle) redirects it, as dup2 on fd 1 does on Unix.
#[cfg(windows)]
fn redirected<T>(to: &File, f: impl FnOnce() -> T) -> T {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::System::Console::{GetStdHandle, SetStdHandle, STD_OUTPUT_HANDLE};
    // SAFETY: the handles are this process's own; `to` outlives the redirection.
    unsafe {
        let saved = GetStdHandle(STD_OUTPUT_HANDLE);
        assert!(SetStdHandle(STD_OUTPUT_HANDLE, to.as_raw_handle() as _) != 0, "SetStdHandle");
        let r = f();
        std::io::stdout().flush().ok();
        SetStdHandle(STD_OUTPUT_HANDLE, saved);
        r
    }
}

fn tempfile() -> File {
    let p = std::env::temp_dir().join(format!("hzengine-out-{}-{}", std::process::id(), nanos()));
    let f = File::options().read(true).write(true).create_new(true).open(&p).unwrap();
    std::fs::remove_file(&p).ok();
    f
}

fn nanos() -> u128 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
}

/// The journal a guest emitted: the line `HZJ:<hex>`.
pub fn journal_of(out: &str) -> Vec<u8> {
    let line = out.lines().find_map(|l| l.strip_prefix("HZJ:")).expect("the guest emitted no journal");
    hex::decode(line.trim()).expect("journal hex")
}
