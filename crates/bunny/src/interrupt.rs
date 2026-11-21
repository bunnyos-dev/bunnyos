//! Ctrl+C.
//!
//! Most of the time Ctrl+C stops bunny straight away, as usual. But at two
//! moments stopping on the spot would leave things half done:
//!
//! - **At a question** in the middle of a switch: bunny has already edited
//!   the config (and maybe the world file), and puts them back when the
//!   answer is no. So Ctrl+C there counts as "no", and the usual clean-up
//!   runs.
//! - **While emerge runs:** emerge gets the Ctrl+C too and stops. bunny
//!   stays to say what got installed and how to undo or finish it.
//!
//! Those stretches are marked with [`Careful`]; a Ctrl+C inside one only sets
//! a flag that the code there checks.

use std::sync::atomic::{AtomicBool, Ordering};

static CAREFUL: AtomicBool = AtomicBool::new(false);
static INTERRUPTED: AtomicBool = AtomicBool::new(false);

extern "C" fn on_sigint(_: libc::c_int) {
    if CAREFUL.load(Ordering::SeqCst) {
        INTERRUPTED.store(true, Ordering::SeqCst);
        return;
    }
    // Only async-signal-safe calls here: finish the line, then stop like a
    // normal Ctrl+C (exit status 130).
    // SAFETY: write and _exit are async-signal-safe.
    unsafe {
        libc::write(libc::STDOUT_FILENO, b"\n".as_ptr().cast(), 1);
        libc::_exit(130);
    }
}

/// Takes over Ctrl+C for the whole run. Without SA_RESTART, so a question
/// waiting for input is woken up.
pub fn install() {
    // SAFETY: a plain sigaction with a handler that only touches atomics,
    // write and _exit.
    unsafe {
        let mut action: libc::sigaction = std::mem::zeroed();
        action.sa_sigaction = on_sigint as *const () as usize;
        action.sa_flags = 0;
        libc::sigemptyset(&mut action.sa_mask);
        libc::sigaction(libc::SIGINT, &action, std::ptr::null_mut());
    }
}

/// While one of these is alive, Ctrl+C sets a flag instead of stopping bunny.
pub struct Careful {
    flag: &'static AtomicBool,
    was: bool,
}

impl Careful {
    pub fn start() -> Careful {
        Self::start_on(&CAREFUL)
    }

    /// The same, on another flag (so tests don't share bunny's own, which
    /// other tests running at the same time set and clear).
    fn start_on(flag: &'static AtomicBool) -> Careful {
        Careful {
            flag,
            was: flag.swap(true, Ordering::SeqCst),
        }
    }
}

impl Drop for Careful {
    fn drop(&mut self) {
        self.flag.store(self.was, Ordering::SeqCst);
    }
}

/// Whether Ctrl+C was pressed during a careful stretch.
pub fn happened() -> bool {
    INTERRUPTED.load(Ordering::SeqCst)
}

/// Reads one line from the terminal. Returns None at the end of input, or
/// when Ctrl+C is pressed while waiting (the question then counts as "no").
pub fn read_line() -> std::io::Result<Option<String>> {
    let _careful = Careful::start();
    let mut bytes = Vec::new();
    loop {
        if happened() {
            return Ok(None);
        }
        let mut byte = 0u8;
        // SAFETY: reads one byte into a valid buffer.
        let n = unsafe { libc::read(libc::STDIN_FILENO, (&mut byte as *mut u8).cast(), 1) };
        match n {
            1 => {
                if byte == b'\n' {
                    return Ok(Some(String::from_utf8_lossy(&bytes).into_owned()));
                }
                bytes.push(byte);
            }
            0 => {
                return Ok((!bytes.is_empty()).then(|| String::from_utf8_lossy(&bytes).into_owned()))
            }
            _ => {
                let err = std::io::Error::last_os_error();
                if err.kind() != std::io::ErrorKind::Interrupted {
                    return Err(err);
                }
                // Woken by a signal: the loop checks whether it was Ctrl+C.
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn careful_stretches_nest() {
        // Its own flag: switch tests running in parallel use CAREFUL.
        static FLAG: AtomicBool = AtomicBool::new(false);
        assert!(!FLAG.load(Ordering::SeqCst));
        {
            let _outer = Careful::start_on(&FLAG);
            {
                let _inner = Careful::start_on(&FLAG);
                assert!(FLAG.load(Ordering::SeqCst));
            }
            assert!(
                FLAG.load(Ordering::SeqCst),
                "the outer stretch is still careful"
            );
        }
        assert!(!FLAG.load(Ordering::SeqCst));
    }
}
