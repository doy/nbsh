use async_std::io::prelude::ReadExt as _;
use async_std::os::unix::process::CommandExt as _;
use async_std::stream::StreamExt as _;
use std::os::unix::io::{AsRawFd as _, FromRawFd as _};
use std::os::unix::process::ExitStatusExt as _;

async fn read_pipeline() -> crate::parse::Pipeline {
    let mut r = unsafe { async_std::fs::File::from_raw_fd(3) };
    let mut s = String::new();
    r.read_to_string(&mut s).await.unwrap();
    crate::parse::Pipeline::parse(&s).unwrap()
}

pub async fn run() {
    let pipeline = read_pipeline().await;

    let mut futures = futures_util::stream::FuturesUnordered::new();
    let mut pg = None;
    let mut stdin = None;
    let last = pipeline.exes().len() - 1;
    for (i, exe) in pipeline.exes().iter().enumerate() {
        let mut cmd = async_std::process::Command::new(exe.exe());
        cmd.args(exe.args());
        if let Some(stdin) = stdin {
            cmd.stdin(unsafe {
                async_std::process::Stdio::from_raw_fd(stdin)
            });
        }
        if i < last {
            let (r, w) =
                nix::unistd::pipe2(nix::fcntl::OFlag::O_CLOEXEC).unwrap();
            stdin = Some(r);
            cmd.stdout(unsafe { async_std::process::Stdio::from_raw_fd(w) });
        }
        let pg_pid = nix::unistd::Pid::from_raw(pg.unwrap_or(0));
        unsafe {
            cmd.pre_exec(move || {
                nix::unistd::setpgid(nix::unistd::Pid::from_raw(0), pg_pid)?;
                Ok(())
            });
        }
        let child = cmd.spawn().unwrap();
        let res = nix::unistd::setpgid(
            nix::unistd::Pid::from_raw(child.id().try_into().unwrap()),
            pg_pid,
        );
        match res {
            Ok(()) => {}
            Err(e) => {
                if e != nix::errno::Errno::EACCES {
                    res.unwrap();
                }
            }
        }
        if pg.is_none() {
            pg = Some(child.id().try_into().unwrap());
        }
        futures.push(async move {
            (child.status_no_drop().await.unwrap(), i == last)
        });
    }

    let pty = std::fs::File::open("/dev/tty").unwrap();
    nix::unistd::tcsetpgrp(
        pty.as_raw_fd(),
        nix::unistd::Pid::from_raw(pg.unwrap()),
    )
    .unwrap();

    let mut final_status = None;
    while let Some((status, last)) = futures.next().await {
        if status.signal() == Some(signal_hook::consts::signal::SIGINT) {
            nix::sys::signal::raise(nix::sys::signal::SIGINT).unwrap();
        }
        if last {
            final_status = Some(status);
        }
    }
    if let Some(code) = final_status.unwrap().code() {
        std::process::exit(code);
    } else {
        std::process::exit(255);
    }
}
