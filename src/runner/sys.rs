use crate::runner::prelude::*;

const PID0: nix::unistd::Pid = nix::unistd::Pid::from_raw(0);

pub fn pipe() -> anyhow::Result<(std::fs::File, std::fs::File)> {
    let (r, w) = nix::unistd::pipe2(nix::fcntl::OFlag::O_CLOEXEC)?;
    // Safety: these file descriptors were just returned by pipe2 above, and
    // are only available in this function, so nothing else can be accessing
    // them
    Ok((unsafe { std::fs::File::from_raw_fd(r) }, unsafe {
        std::fs::File::from_raw_fd(w)
    }))
}

pub fn set_foreground_pg(pg: nix::unistd::Pid) -> anyhow::Result<()> {
    let pty = nix::fcntl::open(
        "/dev/tty",
        nix::fcntl::OFlag::empty(),
        nix::sys::stat::Mode::empty(),
    )?;

    // if a background process calls tcsetpgrp, the kernel will send it
    // SIGTTOU which suspends it. if that background process is the session
    // leader and doesn't have SIGTTOU blocked, the kernel will instead just
    // return ENOTTY from the tcsetpgrp call rather than sending a signal to
    // avoid deadlocking the process. therefore, we need to ensure that
    // SIGTTOU is blocked here.

    // Safety: setting a signal handler to SigIgn is always safe
    unsafe {
        nix::sys::signal::signal(
            nix::sys::signal::Signal::SIGTTOU,
            nix::sys::signal::SigHandler::SigIgn,
        )?;
    }
    let res = nix::unistd::tcsetpgrp(pty, pg);
    // Safety: setting a signal handler to SigDfl is always safe
    unsafe {
        nix::sys::signal::signal(
            nix::sys::signal::Signal::SIGTTOU,
            nix::sys::signal::SigHandler::SigDfl,
        )?;
    }
    res?;

    nix::unistd::close(pty)?;

    nix::sys::signal::kill(neg_pid(pg), nix::sys::signal::Signal::SIGCONT)
        .or_else(|e| {
            // the process group has already exited
            if e == nix::errno::Errno::ESRCH {
                Ok(())
            } else {
                Err(e)
            }
        })?;

    Ok(())
}

pub fn setpgid_child(pg: Option<nix::unistd::Pid>) -> std::io::Result<()> {
    nix::unistd::setpgid(PID0, pg.unwrap_or(PID0))?;
    Ok(())
}

pub fn setpgid_parent(
    pid: nix::unistd::Pid,
    pg: Option<nix::unistd::Pid>,
) -> anyhow::Result<()> {
    nix::unistd::setpgid(pid, pg.unwrap_or(PID0)).or_else(|e| {
        // EACCES means that the child already called exec, but if it did,
        // then it also must have already called setpgid itself, so we don't
        // care. ESRCH means that the process already exited, which is similar
        if e == nix::errno::Errno::EACCES || e == nix::errno::Errno::ESRCH {
            Ok(())
        } else {
            Err(e)
        }
    })?;
    Ok(())
}

pub fn id_to_pid(id: u32) -> nix::unistd::Pid {
    nix::unistd::Pid::from_raw(id.try_into().unwrap())
}

pub fn neg_pid(pid: nix::unistd::Pid) -> nix::unistd::Pid {
    nix::unistd::Pid::from_raw(-pid.as_raw())
}
