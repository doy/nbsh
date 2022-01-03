pub fn pwd() -> anyhow::Result<String> {
    let mut pwd = std::env::current_dir()?.display().to_string();
    if let Ok(home) = std::env::var("HOME") {
        if pwd.starts_with(&home) {
            pwd.replace_range(..home.len(), "~");
        }
    }
    Ok(pwd)
}

pub fn user() -> anyhow::Result<String> {
    Ok(users::get_current_username()
        .ok_or_else(|| anyhow::anyhow!("couldn't get username"))?
        .to_string_lossy()
        .into_owned())
}

#[allow(clippy::unnecessary_wraps)]
pub fn prompt_char() -> anyhow::Result<String> {
    if users::get_current_uid() == 0 {
        Ok("#".into())
    } else {
        Ok("$".into())
    }
}

pub fn hostname() -> anyhow::Result<String> {
    let mut hostname = hostname::get()?.to_string_lossy().into_owned();
    if let Some(idx) = hostname.find('.') {
        hostname.truncate(idx);
    }
    Ok(hostname)
}

#[allow(clippy::unnecessary_wraps)]
pub fn time(offset: time::UtcOffset) -> anyhow::Result<String> {
    Ok(crate::format::time(
        time::OffsetDateTime::now_utc().to_offset(offset),
    ))
}
