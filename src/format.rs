use std::os::unix::process::ExitStatusExt as _;

pub fn exit_status(status: std::process::ExitStatus) -> String {
    if let Some(sig) = status.signal() {
        if let Some(name) = signal_hook::low_level::signal_name(sig) {
            format!("{:4} ", &name[3..])
        } else {
            format!("SIG{} ", sig)
        }
    } else {
        format!("{:03}  ", status.code().unwrap())
    }
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
