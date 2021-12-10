use std::os::unix::process::ExitStatusExt as _;

pub fn exit_status(status: std::process::ExitStatus) -> String {
    status.signal().map_or_else(
        || format!("{:03}  ", status.code().unwrap()),
        |sig| {
            signal_hook::low_level::signal_name(sig).map_or_else(
                || format!("SIG{} ", sig),
                |name| format!("{:4} ", &name[3..]),
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
