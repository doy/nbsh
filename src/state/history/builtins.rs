use std::os::unix::process::ExitStatusExt as _;

pub fn run(
    exe: &crate::parse::Exe,
) -> Option<async_std::process::ExitStatus> {
    match exe.exe() {
        "cd" => Some(cd(exe)),
        _ => None,
    }
}

fn cd(exe: &crate::parse::Exe) -> async_std::process::ExitStatus {
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
