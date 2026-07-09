//! AGT-622: a downstream reader closing the pipe early (`head`, `jq`, a pager)
//! must terminate `wickd` quietly — conventional SIGPIPE death (status 141) or
//! a clean exit — never a Rust panic + backtrace on stderr.
//!
//! The test is deterministic: it hands the child a pipe whose read end is
//! already closed, so the child's very first stdout write hits EPIPE.

#![cfg(unix)]

use std::os::unix::io::FromRawFd;
use std::os::unix::process::ExitStatusExt;
use std::process::{Command, Stdio};

#[test]
fn broken_stdout_pipe_exits_quietly() {
    // pipe() then close the read end before the child exists: every write the
    // child makes to stdout raises SIGPIPE/EPIPE, no races.
    let mut fds = [0i32; 2];
    assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0, "pipe() failed");
    unsafe { libc::close(fds[0]) };
    // SAFETY: fds[1] is a freshly created, valid write-end fd; Stdio takes
    // sole ownership and closes it after spawn.
    let child_stdout = unsafe { Stdio::from_raw_fd(fds[1]) };

    // `strategy list` is fully offline (built-in strategies, no credentials)
    // and prints one JSON object to stdout — enough to trip the broken pipe.
    let out = Command::new(env!("CARGO_BIN_EXE_wickd"))
        .args(["strategy", "list"])
        .stdout(child_stdout)
        .stderr(Stdio::piped())
        .output()
        .expect("failed to spawn wickd");

    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stderr.contains("panicked"),
        "wickd panicked on a broken stdout pipe:\n{stderr}"
    );
    // Conventional SIGPIPE termination (128 + 13 = 141 at the shell) or a
    // clean exit are both acceptable; anything else is a regression.
    assert!(
        out.status.signal() == Some(libc::SIGPIPE) || out.status.success(),
        "unexpected exit status {:?}; stderr:\n{stderr}",
        out.status
    );
}
