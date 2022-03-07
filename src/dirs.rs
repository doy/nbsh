pub fn config_file() -> std::path::PathBuf {
    config_dir().join("config.toml")
}

fn config_dir() -> std::path::PathBuf {
    let project_dirs =
        directories::ProjectDirs::from("", "", "nbsh").unwrap();
    project_dirs.config_dir().to_path_buf()
}
