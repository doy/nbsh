use crate::prelude::*;

pub fn user() -> Result<String> {
    Ok(users::get_current_username()
        .ok_or_else(|| anyhow!("couldn't get username"))?
        .to_string_lossy()
        .into_owned())
}

#[allow(clippy::unnecessary_wraps)]
pub fn prompt_char() -> Result<String> {
    if users::get_current_uid() == 0 {
        Ok("#".into())
    } else {
        Ok("$".into())
    }
}

pub fn hostname() -> Result<String> {
    let mut hostname = hostname::get()?.to_string_lossy().into_owned();
    if let Some(idx) = hostname.find('.') {
        hostname.truncate(idx);
    }
    Ok(hostname)
}

#[allow(clippy::unnecessary_wraps)]
pub fn time(offset: time::UtcOffset) -> Result<String> {
    Ok(crate::format::time(
        time::OffsetDateTime::now_utc().to_offset(offset),
    ))
}

pub fn pid() -> String {
    nix::unistd::getpid().to_string()
}

#[cfg(target_os = "linux")]
#[allow(clippy::unnecessary_wraps)]
pub fn current_exe() -> Result<std::path::PathBuf> {
    Ok("/proc/self/exe".into())
}

#[cfg(not(target_os = "linux"))]
pub fn current_exe() -> Result<std::path::PathBuf> {
    Ok(std::env::current_exe()?)
}

// the time crate is currently unable to get the local offset on unix due to
// soundness concerns, so we have to do it manually/:
//
// https://github.com/time-rs/time/issues/380
pub fn get_offset() -> time::UtcOffset {
    let offset_str =
        std::process::Command::new("date").args(&["+%:z"]).output();
    if let Ok(offset_str) = offset_str {
        let offset_str = String::from_utf8(offset_str.stdout).unwrap();
        time::UtcOffset::parse(
            offset_str.trim(),
            &time::format_description::parse("[offset_hour]:[offset_minute]")
                .unwrap(),
        )
        .unwrap_or(time::UtcOffset::UTC)
    } else {
        time::UtcOffset::UTC
    }
}
