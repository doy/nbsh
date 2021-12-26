use std::os::unix::process::ExitStatusExt as _;

type Builtin = &'static (dyn for<'a> Fn(
    &'a crate::parse::Exe,
    &'a super::ProcessEnv,
) -> std::pin::Pin<
    Box<
        dyn std::future::Future<Output = std::process::ExitStatus>
            + Sync
            + Send
            + 'a,
    >,
> + Sync
              + Send);

static BUILTINS: once_cell::sync::Lazy<
    std::collections::HashMap<&'static str, Builtin>,
> = once_cell::sync::Lazy::new(|| {
    // all this does is convince the type system to do the right thing, i
    // don't think there's any way to just do it directly through annotations
    // or casts or whatever
    fn coerce_builtin<F>(f: &'static F) -> Builtin
    where
        F: for<'a> Fn(
                &'a crate::parse::Exe,
                &'a super::ProcessEnv,
            ) -> std::pin::Pin<
                Box<
                    dyn std::future::Future<Output = std::process::ExitStatus>
                        + Sync
                        + Send
                        + 'a,
                >,
            > + Sync
            + Send
            + 'static,
    {
        f
    }

    let mut builtins = std::collections::HashMap::new();
    builtins.insert("cd", coerce_builtin(&|exe, env| Box::pin(cd(exe, env))));
    builtins
        .insert("and", coerce_builtin(&|exe, env| Box::pin(and(exe, env))));
    builtins.insert("or", coerce_builtin(&|exe, env| Box::pin(or(exe, env))));
    builtins.insert(
        "command",
        coerce_builtin(&|exe, env| Box::pin(command(exe, env))),
    );
    builtins.insert(
        "builtin",
        coerce_builtin(&|exe, env| Box::pin(builtin(exe, env))),
    );
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
    let code = match std::env::set_current_dir(&dir) {
        Ok(()) => 0,
        Err(e) => {
            env.write_vt(
                format!(
                    "{}: {}: {}",
                    exe.exe(),
                    crate::format::io_error(&e),
                    dir.display()
                )
                .as_bytes(),
            )
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
        super::run_exe(&exe, env).await
    } else {
        *env.latest_status()
    }
}

async fn or(
    exe: &crate::parse::Exe,
    env: &super::ProcessEnv,
) -> async_std::process::ExitStatus {
    let exe = exe.shift();
    if env.latest_status().success() {
        *env.latest_status()
    } else {
        super::run_exe(&exe, env).await
    }
}

async fn command(
    exe: &crate::parse::Exe,
    env: &super::ProcessEnv,
) -> async_std::process::ExitStatus {
    let exe = exe.shift();
    super::run_binary(&exe, env).await;
    *env.latest_status()
}

async fn builtin(
    exe: &crate::parse::Exe,
    env: &super::ProcessEnv,
) -> async_std::process::ExitStatus {
    let exe = exe.shift();
    run(&exe, env).await;
    *env.latest_status()
}

fn home() -> std::path::PathBuf {
    std::env::var_os("HOME").unwrap().into()
}
