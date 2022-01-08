use crate::pipeline::prelude::*;

pub mod command;
pub use command::{Child, Command};

type Builtin = &'static (dyn for<'a> Fn(
    crate::parse::Exe,
    &'a Env,
    command::Io,
) -> anyhow::Result<command::Child<'a>>
              + Sync
              + Send);

#[allow(clippy::as_conversions)]
static BUILTINS: once_cell::sync::Lazy<
    std::collections::HashMap<&'static str, Builtin>,
> = once_cell::sync::Lazy::new(|| {
    let mut builtins = std::collections::HashMap::new();
    builtins.insert("cd", &cd as Builtin);
    builtins.insert("setenv", &setenv);
    builtins.insert("unsetenv", &unsetenv);
    builtins.insert("echo", &echo);
    builtins.insert("and", &and);
    builtins.insert("or", &or);
    builtins.insert("command", &command);
    builtins.insert("builtin", &builtin);
    builtins
});

macro_rules! bail {
    ($io:expr, $exe:expr, $msg:expr $(,)?) => {
        $io.write_stderr(
            format!("{}: {}\n", $exe.exe().display(), $msg).as_bytes()
        )
        .await
        .unwrap();
        return std::process::ExitStatus::from_raw(1 << 8);
    };
    ($io:expr, $exe:expr, $msg:expr, $($arg:tt)*) => {
        $io.write_stderr(
            format!("{}: ", $exe.exe().display()).as_bytes()
        )
        .await
        .unwrap();
        $io.write_stderr(format!($msg, $($arg)*).as_bytes())
            .await
            .unwrap();
        $io.write_stderr(b"\n").await.unwrap();
        return std::process::ExitStatus::from_raw(1 << 8);
    };
}

// clippy can't tell that the type is necessary
#[allow(clippy::unnecessary_wraps)]
fn cd(
    exe: crate::parse::Exe,
    env: &Env,
    io: command::Io,
) -> anyhow::Result<command::Child> {
    async fn async_cd(
        exe: crate::parse::Exe,
        _env: &Env,
        io: command::Io,
    ) -> std::process::ExitStatus {
        let dir = exe.args().get(0).map_or("", String::as_str);
        let dir = if dir.is_empty() {
            if let Some(dir) = home(None) {
                dir
            } else {
                bail!(io, exe, "couldn't find current user");
            }
        } else if dir.starts_with('~') {
            let path: std::path::PathBuf = dir.into();
            if let std::path::Component::Normal(prefix) =
                path.components().next().unwrap()
            {
                let prefix_bytes = prefix.as_bytes();
                let name = if prefix_bytes == b"~" {
                    None
                } else {
                    Some(std::ffi::OsStr::from_bytes(&prefix_bytes[1..]))
                };
                if let Some(home) = home(name) {
                    home.join(path.strip_prefix(prefix).unwrap())
                } else {
                    bail!(
                        io,
                        exe,
                        "no such user: {}",
                        name.map(std::ffi::OsStr::to_string_lossy)
                            .as_ref()
                            .unwrap_or(&std::borrow::Cow::Borrowed(
                                "(deleted)"
                            ))
                    );
                }
            } else {
                unreachable!()
            }
        } else {
            dir.into()
        };
        if let Err(e) = std::env::set_current_dir(&dir) {
            bail!(
                io,
                exe,
                "{}: {}",
                crate::format::io_error(&e),
                dir.display()
            );
        }
        async_std::process::ExitStatus::from_raw(0)
    }

    Ok(command::Child::new_fut(async move {
        async_cd(exe, env, io).await
    }))
}

#[allow(clippy::unnecessary_wraps)]
fn setenv(
    exe: crate::parse::Exe,
    env: &Env,
    io: command::Io,
) -> anyhow::Result<command::Child> {
    async fn async_setenv(
        exe: crate::parse::Exe,
        _env: &Env,
        io: command::Io,
    ) -> std::process::ExitStatus {
        let k = if let Some(k) = exe.args().get(0).map(String::as_str) {
            k
        } else {
            bail!(io, exe, "usage: setenv key value");
        };
        let v = if let Some(v) = exe.args().get(1).map(String::as_str) {
            v
        } else {
            bail!(io, exe, "usage: setenv key value");
        };

        std::env::set_var(k, v);
        async_std::process::ExitStatus::from_raw(0)
    }

    Ok(command::Child::new_fut(async move {
        async_setenv(exe, env, io).await
    }))
}

#[allow(clippy::unnecessary_wraps)]
fn unsetenv(
    exe: crate::parse::Exe,
    env: &Env,
    io: command::Io,
) -> anyhow::Result<command::Child> {
    async fn async_unsetenv(
        exe: crate::parse::Exe,
        _env: &Env,
        io: command::Io,
    ) -> std::process::ExitStatus {
        let k = if let Some(k) = exe.args().get(0).map(String::as_str) {
            k
        } else {
            bail!(io, exe, "usage: unsetenv key");
        };

        std::env::remove_var(k);
        async_std::process::ExitStatus::from_raw(0)
    }

    Ok(command::Child::new_fut(async move {
        async_unsetenv(exe, env, io).await
    }))
}

// clippy can't tell that the type is necessary
#[allow(clippy::unnecessary_wraps)]
// mostly just for testing and ensuring that builtins work, i'll likely remove
// this later, since the binary seems totally fine
fn echo(
    exe: crate::parse::Exe,
    env: &Env,
    io: command::Io,
) -> anyhow::Result<command::Child> {
    async fn async_echo(
        exe: crate::parse::Exe,
        _env: &Env,
        io: command::Io,
    ) -> std::process::ExitStatus {
        macro_rules! write_stdout {
            ($bytes:expr) => {
                if let Err(e) = io.write_stdout($bytes).await {
                    io.write_stderr(format!("echo: {}", e).as_bytes())
                        .await
                        .unwrap();
                    return async_std::process::ExitStatus::from_raw(1 << 8);
                }
            };
        }
        let count = exe.args().len();
        for (i, arg) in exe.args().iter().enumerate() {
            write_stdout!(arg.as_bytes());
            if i == count - 1 {
                write_stdout!(b"\n");
            } else {
                write_stdout!(b" ");
            }
        }

        async_std::process::ExitStatus::from_raw(0)
    }

    Ok(command::Child::new_fut(async move {
        async_echo(exe, env, io).await
    }))
}

fn and(
    mut exe: crate::parse::Exe,
    env: &Env,
    io: command::Io,
) -> anyhow::Result<command::Child> {
    exe.shift();
    if env.latest_status().success() {
        let mut cmd = crate::pipeline::Command::new(exe);
        io.setup_command(&mut cmd);
        Ok(command::Child::new_wrapped(cmd.spawn(env)?))
    } else {
        let status = *env.latest_status();
        Ok(command::Child::new_fut(async move { status }))
    }
}

fn or(
    mut exe: crate::parse::Exe,
    env: &Env,
    io: command::Io,
) -> anyhow::Result<command::Child> {
    exe.shift();
    if env.latest_status().success() {
        let status = *env.latest_status();
        Ok(command::Child::new_fut(async move { status }))
    } else {
        let mut cmd = crate::pipeline::Command::new(exe);
        io.setup_command(&mut cmd);
        Ok(command::Child::new_wrapped(cmd.spawn(env)?))
    }
}

fn command(
    mut exe: crate::parse::Exe,
    env: &Env,
    io: command::Io,
) -> anyhow::Result<command::Child> {
    exe.shift();
    let mut cmd = crate::pipeline::Command::new_binary(exe);
    io.setup_command(&mut cmd);
    Ok(command::Child::new_wrapped(cmd.spawn(env)?))
}

fn builtin(
    mut exe: crate::parse::Exe,
    env: &Env,
    io: command::Io,
) -> anyhow::Result<command::Child> {
    exe.shift();
    let mut cmd = crate::pipeline::Command::new_builtin(exe);
    io.setup_command(&mut cmd);
    Ok(command::Child::new_wrapped(cmd.spawn(env)?))
}

fn home(user: Option<&std::ffi::OsStr>) -> Option<std::path::PathBuf> {
    let user = user.map_or_else(
        || users::get_user_by_uid(users::get_current_uid()),
        users::get_user_by_name,
    );
    user.map(|user| user.home_dir().to_path_buf())
}
