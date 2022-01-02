use std::io::Read as _;
use std::os::unix::io::FromRawFd as _;
use std::os::unix::process::CommandExt as _;

const PID0: nix::unistd::Pid = nix::unistd::Pid::from_raw(0);

pub fn run() -> anyhow::Result<i32> {
    let pipeline = read_pipeline()?;
    let mut cmds: Vec<_> = pipeline
        .exes()
        .iter()
        .map(|exe| {
            let mut cmd = std::process::Command::new(exe.exe());
            cmd.args(exe.args());
            cmd
        })
        .collect();
    for i in 0..(cmds.len() - 1) {
        let (r, w) = pipe()?;
        cmds[i].stdout(w);
        cmds[i + 1].stdin(r);
    }

    let mut children = vec![];

    // Safety: setpgid is an async-signal-safe function
    unsafe {
        cmds[0].pre_exec(|| {
            setpgid_child(PID0)?;
            Ok(())
        });
    }
    let leader = cmds[0].spawn()?;
    let pg_pid = id_to_pid(leader.id());
    setpgid_parent(pg_pid, PID0)?;
    set_foreground_pg(pg_pid)?;
    children.push(leader);

    for cmd in &mut cmds[1..] {
        // Safety: setpgid is an async-signal-safe function
        unsafe {
            cmd.pre_exec(move || {
                setpgid_child(pg_pid)?;
                Ok(())
            });
        }
        let child = cmd.spawn()?;
        let child_pid = id_to_pid(child.id());
        children.push(child);
        setpgid_parent(child_pid, pg_pid)?;
    }
    // ensure that we don't keep the pipes open past when the children exit
    drop(cmds);

    let last_pid = id_to_pid(children[children.len() - 1].id());
    let mut children: std::collections::HashMap<
        nix::unistd::Pid,
        std::process::Child,
    > = children
        .into_iter()
        .map(|child| (id_to_pid(child.id()), child))
        .collect();
    let mut final_code = None;
    let mut final_signal = None;
    while !children.is_empty() {
        match nix::sys::wait::waitpid(neg_pid(pg_pid), None)? {
            nix::sys::wait::WaitStatus::Exited(pid, code) => {
                if pid == last_pid {
                    final_code = Some(code);
                }
                children.remove(&pid);
            }
            nix::sys::wait::WaitStatus::Signaled(pid, signal, _) => {
                if signal == nix::sys::signal::Signal::SIGINT {
                    nix::sys::signal::raise(nix::sys::signal::SIGINT)?;
                }
                if pid == last_pid {
                    final_signal = Some(signal);
                }
                children.remove(&pid);
            }
            _ => {}
        }
    }
    if let Some(signal) = final_signal {
        nix::sys::signal::raise(signal)?;
    }
    Ok(final_code.unwrap())
}

fn read_pipeline() -> anyhow::Result<crate::parse::Pipeline> {
    // Safety: this code is only called by crate::history::run_pipeline, which
    // passes data through on fd 3, and which will not spawn this process
    // unless the pipe was successfully opened on that fd
    let mut fd3 = unsafe { std::fs::File::from_raw_fd(3) };
    let mut pipeline = String::new();
    fd3.read_to_string(&mut pipeline)?;
    let ast = crate::parse::Pipeline::parse(&pipeline)?;
    Ok(ast)
}

fn pipe() -> anyhow::Result<(std::fs::File, std::fs::File)> {
    let (r, w) = nix::unistd::pipe2(nix::fcntl::OFlag::O_CLOEXEC)?;
    // Safety: these file descriptors were just returned by pipe2 above, which
    // means they must be valid otherwise that call would have returned an
    // error
    Ok((unsafe { std::fs::File::from_raw_fd(r) }, unsafe {
        std::fs::File::from_raw_fd(w)
    }))
}

fn set_foreground_pg(pg: nix::unistd::Pid) -> anyhow::Result<()> {
    let pty = nix::fcntl::open(
        "/dev/tty",
        nix::fcntl::OFlag::empty(),
        nix::sys::stat::Mode::empty(),
    )?;
    nix::unistd::tcsetpgrp(pty, pg)?;
    nix::unistd::close(pty)?;
    nix::sys::signal::kill(neg_pid(pg), nix::sys::signal::Signal::SIGCONT)?;
    Ok(())
}

fn setpgid_child(pg: nix::unistd::Pid) -> std::io::Result<()> {
    nix::unistd::setpgid(id_to_pid(0), pg)?;
    Ok(())
}

fn setpgid_parent(
    pid: nix::unistd::Pid,
    pg: nix::unistd::Pid,
) -> anyhow::Result<()> {
    nix::unistd::setpgid(pid, pg).or_else(|e| {
        // EACCES means that the child already called exec, but if it did,
        // then it also must have already called setpgid itself, so we don't
        // care
        if e == nix::errno::Errno::EACCES {
            Ok(())
        } else {
            Err(e)
        }
    })?;
    Ok(())
}

fn id_to_pid(id: u32) -> nix::unistd::Pid {
    nix::unistd::Pid::from_raw(id.try_into().unwrap())
}

fn neg_pid(pid: nix::unistd::Pid) -> nix::unistd::Pid {
    nix::unistd::Pid::from_raw(-pid.as_raw())
}
