static PROJECT_DIRS: once_cell::sync::Lazy<directories::ProjectDirs> =
    once_cell::sync::Lazy::new(|| {
        directories::ProjectDirs::from("", "", "nbsh").unwrap()
    });

pub fn config_file() -> std::path::PathBuf {
    config_dir().join("config.toml")
}

pub fn history_file() -> std::path::PathBuf {
    data_dir().join("history")
}

fn config_dir() -> std::path::PathBuf {
    PROJECT_DIRS.config_dir().to_path_buf()
}

fn data_dir() -> std::path::PathBuf {
    PROJECT_DIRS.data_dir().to_path_buf()
}
