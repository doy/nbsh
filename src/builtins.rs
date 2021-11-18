pub fn is(exe: &str) -> bool {
    matches!(exe, "cd")
}

pub fn run(exe: &str, args: &[String]) -> u8 {
    match exe {
        "cd" => {
            impls::cd(args.iter().map(|s| s.as_ref()).next().unwrap_or(""))
        }
        _ => unreachable!(),
    }
}

mod impls {
    pub fn cd(dir: &str) -> u8 {
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
                    return 1;
                }
            } else {
                unreachable!()
            }
        } else {
            dir.into()
        };
        match std::env::set_current_dir(dir) {
            Ok(()) => 0,
            Err(_) => 1,
        }
    }

    fn home() -> std::path::PathBuf {
        std::env::var_os("HOME").unwrap().into()
    }
}
