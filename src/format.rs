use std::os::unix::process::ExitStatusExt as _;

pub fn exit_status(status: std::process::ExitStatus) -> String {
    status.signal().map_or_else(
        || format!("{:03}  ", status.code().unwrap()),
        |sig| {
            nix::sys::signal::Signal::try_from(sig).map_or_else(
                |_| format!("SIG{} ", sig),
                |sig| format!("{:4} ", &sig.as_str()[3..]),
            )
        },
    )
}

pub fn time(time: time::OffsetDateTime) -> String {
    let format =
        time::format_description::parse("[hour]:[minute]:[second]").unwrap();
    time.format(&format).unwrap()
}

pub fn duration(dur: std::time::Duration) -> String {
    let secs = dur.as_secs();
    let nanos = dur.subsec_nanos();
    if secs > 60 {
        let mins = secs / 60;
        let secs = secs - mins * 60;
        format!("{}m{}s", mins, secs)
    } else if secs > 9 {
        format!("{}.{:02}s", secs, nanos / 10_000_000)
    } else if secs > 0 {
        format!("{}.{:03}s", secs, nanos / 1_000_000)
    } else if nanos >= 1_000_000 {
        format!("{}ms", nanos / 1_000_000)
    } else if nanos >= 1_000 {
        format!("{}us", nanos / 1_000)
    } else {
        format!("{}ns", nanos)
    }
}

pub fn io_error(e: &std::io::Error) -> String {
    let mut s = format!("{}", e);
    if e.raw_os_error().is_some() {
        let i = s.rfind('(').unwrap();
        s.truncate(i - 1);
    }
    s
}
