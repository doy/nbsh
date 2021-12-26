use std::os::unix::process::ExitStatusExt as _;

use std::future::Future;
use std::pin::Pin;

// i hate all of this so much
type Builtin = Box<
    dyn for<'a> Fn(
            &'a crate::parse::Exe,
            &'a super::ProcessEnv,
        ) -> Pin<
            Box<
                dyn Future<Output = std::process::ExitStatus>
                    + Sync
                    + Send
                    + 'a,
            >,
        > + Sync
        + Send,
>;

static BUILTINS: once_cell::sync::Lazy<
    std::collections::HashMap<&'static str, Builtin>,
> = once_cell::sync::Lazy::new(|| {
    fn box_builtin<F>(f: F) -> Builtin
    where
        F: for<'a> Fn(
                &'a crate::parse::Exe,
                &'a super::ProcessEnv,
            ) -> Pin<
                Box<
                    dyn Future<Output = std::process::ExitStatus>
                        + Sync
                        + Send
                        + 'a,
                >,
            > + Sync
            + Send
            + 'static,
    {
        Box::new(f)
    }

    let mut builtins = std::collections::HashMap::new();
    builtins.insert("cd", box_builtin(|exe, env| Box::pin(cd(exe, env))));
    builtins.insert("and", box_builtin(|exe, env| Box::pin(and(exe, env))));
    builtins.insert("or", box_builtin(|exe, env| Box::pin(or(exe, env))));
    builtins
});

pub async fn run(
    exe: &crate::parse::Exe,
    env: &super::ProcessEnv,
) -> Option<async_std::process::ExitStatus> {
    if let Some(f) = BUILTINS.get(exe.exe()) {
        Some(f(exe, env).await)
    } else {
        None
    }
}

async fn cd(
    exe: &crate::parse::Exe,
    env: &super::ProcessEnv,
) -> async_std::process::ExitStatus {
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
                env.write_vt(b"unimplemented").await;
                return async_std::process::ExitStatus::from_raw(1 << 8);
            }
        } else {
            unreachable!()
        }
    } else {
        dir.into()
    };
    let code = match std::env::set_current_dir(dir) {
        Ok(()) => 0,
        Err(e) => {
            env.write_vt(format!("{}: {}", exe.exe(), e).as_bytes())
                .await;
            1
        }
    };
    async_std::process::ExitStatus::from_raw(code << 8)
}

async fn and(
    exe: &crate::parse::Exe,
    env: &super::ProcessEnv,
) -> async_std::process::ExitStatus {
    let exe = exe.shift();
    if env.latest_status().success() {
        super::run_exe(&exe, env).await;
    }
    *env.latest_status()
}

async fn or(
    exe: &crate::parse::Exe,
    env: &super::ProcessEnv,
) -> async_std::process::ExitStatus {
    let exe = exe.shift();
    if !env.latest_status().success() {
        super::run_exe(&exe, env).await;
    }
    *env.latest_status()
}

fn home() -> std::path::PathBuf {
    std::env::var_os("HOME").unwrap().into()
}
