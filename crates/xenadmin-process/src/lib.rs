//! The narrowly scoped post-fork adapter needed by the offline image sandbox.
use std::{io, os::unix::process::CommandExt, process::Command};

/// Prevent all inherited descriptors above stderr from surviving the next exec.
///
/// This is installed before spawning and runs only in the forked child. A kernel
/// without CLOSE_RANGE_CLOEXEC refuses the spawn; there is no enumeration fallback.
#[allow(unsafe_code)]
pub fn close_inherited_on_exec(command: &mut Command) {
    // SAFETY: the post-fork closure uses only close_range and errno access, with
    // no allocation, locks, environment access, or parent descriptor mutations.
    // CLOEXEC preserves Rust's error-reporting pipe until exec; fd 0 (the sandbox
    // policy input) and output pipes 1/2 are deliberately retained.
    unsafe {
        command.pre_exec(|| {
            if libc::close_range(3, u32::MAX, libc::CLOSE_RANGE_CLOEXEC as libc::c_int) == 0 {
                Ok(())
            } else {
                Err(io::Error::last_os_error())
            }
        });
    }
}
