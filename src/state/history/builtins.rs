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

fn box_builtin<F: 'static>(f: F) -> Builtin
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
        + Send,
{
    Box::new(move |exe, env| Box::pin(f(exe, env)))
}

static BUILTINS: once_cell::sync::Lazy<
    std::collections::HashMap<&'static str, Builtin>,
> = once_cell::sync::Lazy::new(|| {
    let mut builtins = std::collections::HashMap::new();
    builtins
        .insert("cd", box_builtin(move |exe, env| Box::pin(cd(exe, env))));
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
    _: &super::ProcessEnv,
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
        Err(_) => 1,
    };
    async_std::process::ExitStatus::from_raw(code << 8)
}

fn home() -> std::path::PathBuf {
    std::env::var_os("HOME").unwrap().into()
}
