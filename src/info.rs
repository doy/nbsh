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
