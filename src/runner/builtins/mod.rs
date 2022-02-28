use crate::runner::prelude::*;

pub mod command;
pub use command::{Child, Command, File, Io};

type Builtin = &'static (dyn for<'a> Fn(
    crate::parse::Exe,
    &'a Env,
    command::Cfg,
) -> Result<command::Child>
              + Sync
              + Send);

#[allow(clippy::as_conversions)]
static BUILTINS: once_cell::sync::Lazy<
    std::collections::HashMap<&'static str, Builtin>,
> = once_cell::sync::Lazy::new(|| {
    let mut builtins = std::collections::HashMap::new();
    builtins.insert("cd", &cd as Builtin);
    builtins.insert("set", &set);
    builtins.insert("unset", &unset);
    builtins.insert("echo", &echo);
    builtins.insert("read", &read);
    builtins.insert("and", &and);
    builtins.insert("or", &or);
    builtins.insert("command", &command);
    builtins.insert("builtin", &builtin);
    builtins
});

macro_rules! bail {
    ($cfg:expr, $exe:expr, $msg:expr $(,)?) => {
        $cfg.io().write_stderr(
            format!("{}: {}\n", $exe.exe().display(), $msg).as_bytes()
        )
        .unwrap();
        return std::process::ExitStatus::from_raw(1 << 8);
    };
    ($cfg:expr, $exe:expr, $msg:expr, $($arg:tt)*) => {
        $cfg.io().write_stderr(
            format!("{}: ", $exe.exe().display()).as_bytes()
        )
        .unwrap();
        $cfg.io().write_stderr(format!($msg, $($arg)*).as_bytes())
            .unwrap();
        $cfg.io().write_stderr(b"\n").unwrap();
        return std::process::ExitStatus::from_raw(1 << 8);
    };
}

// clippy can't tell that the type is necessary
#[allow(clippy::unnecessary_wraps)]
fn cd(
    exe: crate::parse::Exe,
    env: &Env,
    cfg: command::Cfg,
) -> Result<command::Child> {
    let prev_pwd = env.prev_pwd();
    let home = env.var("HOME");
    Ok(command::Child::new_task(move || {
        let dir = if let Some(dir) = exe.args().get(0) {
            if dir.is_empty() {
                ".".to_string().into()
            } else if dir == "-" {
                prev_pwd
            } else {
                dir.into()
            }
        } else {
            let dir = home;
            if let Some(dir) = dir {
                dir.into()
            } else {
                bail!(cfg, exe, "could not find home directory");
            }
        };
        if let Err(e) = std::env::set_current_dir(&dir) {
            bail!(
                cfg,
                exe,
                "{}: {}",
                crate::format::io_error(&e),
                dir.display()
            );
        }
        std::process::ExitStatus::from_raw(0)
    }))
}

#[allow(clippy::unnecessary_wraps)]
fn set(
    exe: crate::parse::Exe,
    _env: &Env,
    cfg: command::Cfg,
) -> Result<command::Child> {
    Ok(command::Child::new_task(move || {
        let k = if let Some(k) = exe.args().get(0).map(String::as_str) {
            k
        } else {
            bail!(cfg, exe, "usage: set key value");
        };
        let v = if let Some(v) = exe.args().get(1).map(String::as_str) {
            v
        } else {
            bail!(cfg, exe, "usage: set key value");
        };

        std::env::set_var(k, v);
        std::process::ExitStatus::from_raw(0)
    }))
}

#[allow(clippy::unnecessary_wraps)]
fn unset(
    exe: crate::parse::Exe,
    _env: &Env,
    cfg: command::Cfg,
) -> Result<command::Child> {
    Ok(command::Child::new_task(move || {
        let k = if let Some(k) = exe.args().get(0).map(String::as_str) {
            k
        } else {
            bail!(cfg, exe, "usage: unset key");
        };

        std::env::remove_var(k);
        std::process::ExitStatus::from_raw(0)
    }))
}

// clippy can't tell that the type is necessary
#[allow(clippy::unnecessary_wraps)]
// mostly just for testing and ensuring that builtins work, i'll likely remove
// this later, since the binary seems totally fine
fn echo(
    exe: crate::parse::Exe,
    _env: &Env,
    cfg: command::Cfg,
) -> Result<command::Child> {
    Ok(command::Child::new_task(move || {
        macro_rules! write_stdout {
            ($bytes:expr) => {
                if let Err(e) = cfg.io().write_stdout($bytes) {
                    cfg.io()
                        .write_stderr(format!("echo: {}", e).as_bytes())
                        .unwrap();
                    return std::process::ExitStatus::from_raw(1 << 8);
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

        std::process::ExitStatus::from_raw(0)
    }))
}

#[allow(clippy::unnecessary_wraps)]
fn read(
    exe: crate::parse::Exe,
    _env: &Env,
    cfg: command::Cfg,
) -> Result<command::Child> {
    Ok(command::Child::new_task(move || {
        let var = if let Some(var) = exe.args().get(0).map(String::as_str) {
            var
        } else {
            bail!(cfg, exe, "usage: read var");
        };

        let (val, done) = match cfg.io().read_line_stdin() {
            Ok((line, done)) => (line, done),
            Err(e) => {
                bail!(cfg, exe, e);
            }
        };

        std::env::set_var(var, val);
        std::process::ExitStatus::from_raw(if done { 1 << 8 } else { 0 })
    }))
}

fn and(
    mut exe: crate::parse::Exe,
    env: &Env,
    cfg: command::Cfg,
) -> Result<command::Child> {
    exe.shift();
    if env.latest_status().success() {
        let mut cmd = crate::runner::Command::new(exe, cfg.io().clone());
        cfg.setup_command(&mut cmd);
        Ok(command::Child::new_wrapped(cmd.spawn(env)?))
    } else {
        let status = env.latest_status();
        Ok(command::Child::new_task(move || status))
    }
}

fn or(
    mut exe: crate::parse::Exe,
    env: &Env,
    cfg: command::Cfg,
) -> Result<command::Child> {
    exe.shift();
    if env.latest_status().success() {
        let status = env.latest_status();
        Ok(command::Child::new_task(move || status))
    } else {
        let mut cmd = crate::runner::Command::new(exe, cfg.io().clone());
        cfg.setup_command(&mut cmd);
        Ok(command::Child::new_wrapped(cmd.spawn(env)?))
    }
}

fn command(
    mut exe: crate::parse::Exe,
    env: &Env,
    cfg: command::Cfg,
) -> Result<command::Child> {
    exe.shift();
    let mut cmd = crate::runner::Command::new_binary(&exe);
    cfg.setup_command(&mut cmd);
    Ok(command::Child::new_wrapped(cmd.spawn(env)?))
}

fn builtin(
    mut exe: crate::parse::Exe,
    env: &Env,
    cfg: command::Cfg,
) -> Result<command::Child> {
    exe.shift();
    let mut cmd = crate::runner::Command::new_builtin(exe, cfg.io().clone());
    cfg.setup_command(&mut cmd);
    Ok(command::Child::new_wrapped(cmd.spawn(env)?))
}
