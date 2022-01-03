use std::os::unix::process::ExitStatusExt as _;

pub mod command;
pub use command::{Child, Command};

type Builtin = &'static (dyn Fn(
    &crate::parse::Exe,
    &crate::env::Env,
    command::Io,
) -> anyhow::Result<command::Child>
              + Sync
              + Send);

#[allow(clippy::as_conversions)]
static BUILTINS: once_cell::sync::Lazy<
    std::collections::HashMap<&'static str, Builtin>,
> = once_cell::sync::Lazy::new(|| {
    let mut builtins = std::collections::HashMap::new();
    builtins.insert("cd", &cd as Builtin);
    builtins.insert("echo", &echo);
    builtins.insert("and", &and);
    builtins.insert("or", &or);
    builtins.insert("command", &command);
    builtins.insert("builtin", &builtin);
    builtins
});

// clippy can't tell that the type is necessary
#[allow(clippy::unnecessary_wraps)]
fn cd(
    exe: &crate::parse::Exe,
    env: &crate::env::Env,
    io: command::Io,
) -> anyhow::Result<command::Child> {
    async fn async_cd(
        exe: &crate::parse::Exe,
        _env: &crate::env::Env,
        io: command::Io,
    ) -> std::process::ExitStatus {
        let dir = exe
            .args()
            .into_iter()
            .map(std::convert::AsRef::as_ref)
            .next()
            .unwrap_or("");

        let dir = if dir.is_empty() {
            home()
        } else if dir.starts_with('~') {
            let path: std::path::PathBuf = dir.into();
            if let std::path::Component::Normal(prefix) =
                path.components().next().unwrap()
            {
                if prefix.to_str() == Some("~") {
                    home().join(path.strip_prefix(prefix).unwrap())
                } else {
                    // TODO
                    io.write_stderr(b"unimplemented\n").await.unwrap();
                    return async_std::process::ExitStatus::from_raw(1 << 8);
                }
            } else {
                unreachable!()
            }
        } else {
            dir.into()
        };
        let code = match std::env::set_current_dir(&dir) {
            Ok(()) => 0,
            Err(e) => {
                io.write_stderr(
                    format!(
                        "{}: {}: {}\n",
                        exe.exe(),
                        crate::format::io_error(&e),
                        dir.display()
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
                1
            }
        };
        async_std::process::ExitStatus::from_raw(code << 8)
    }

    let exe = exe.clone();
    let env = env.clone();
    Ok(command::Child::new_fut(async move {
        async_cd(&exe, &env, io).await
    }))
}

// clippy can't tell that the type is necessary
#[allow(clippy::unnecessary_wraps)]
// mostly just for testing and ensuring that builtins work, i'll likely remove
// this later, since the binary seems totally fine
fn echo(
    exe: &crate::parse::Exe,
    env: &crate::env::Env,
    io: command::Io,
) -> anyhow::Result<command::Child> {
    async fn async_echo(
        exe: &crate::parse::Exe,
        _env: &crate::env::Env,
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
        let count = exe.args().count();
        for (i, arg) in exe.args().enumerate() {
            write_stdout!(arg.as_bytes());
            if i == count - 1 {
                write_stdout!(b"\n");
            } else {
                write_stdout!(b" ");
            }
        }

        async_std::process::ExitStatus::from_raw(0)
    }

    let exe = exe.clone();
    let env = env.clone();
    Ok(command::Child::new_fut(async move {
        async_echo(&exe, &env, io).await
    }))
}

fn and(
    exe: &crate::parse::Exe,
    env: &crate::env::Env,
    io: command::Io,
) -> anyhow::Result<command::Child> {
    let exe = exe.shift();
    if env.latest_status().success() {
        let mut cmd = crate::pipeline::Command::new(&exe);
        io.setup_command(&mut cmd);
        Ok(command::Child::new_wrapped(cmd.spawn(env)?))
    } else {
        let env = env.clone();
        Ok(command::Child::new_fut(async move { *env.latest_status() }))
    }
}

fn or(
    exe: &crate::parse::Exe,
    env: &crate::env::Env,
    io: command::Io,
) -> anyhow::Result<command::Child> {
    let exe = exe.shift();
    if env.latest_status().success() {
        let env = env.clone();
        Ok(command::Child::new_fut(async move { *env.latest_status() }))
    } else {
        let mut cmd = crate::pipeline::Command::new(&exe);
        io.setup_command(&mut cmd);
        Ok(command::Child::new_wrapped(cmd.spawn(env)?))
    }
}

fn command(
    exe: &crate::parse::Exe,
    env: &crate::env::Env,
    io: command::Io,
) -> anyhow::Result<command::Child> {
    let exe = exe.shift();
    let mut cmd = crate::pipeline::Command::new_binary(&exe);
    io.setup_command(&mut cmd);
    Ok(command::Child::new_wrapped(cmd.spawn(env)?))
}

fn builtin(
    exe: &crate::parse::Exe,
    env: &crate::env::Env,
    io: command::Io,
) -> anyhow::Result<command::Child> {
    let exe = exe.shift();
    let mut cmd = crate::pipeline::Command::new_builtin(&exe);
    io.setup_command(&mut cmd);
    Ok(command::Child::new_wrapped(cmd.spawn(env)?))
}

fn home() -> std::path::PathBuf {
    std::env::var_os("HOME").unwrap().into()
}
