pub type Mutex<T> = async_std::sync::Arc<async_std::sync::Mutex<T>>;

pub fn mutex<T>(t: T) -> Mutex<T> {
    async_std::sync::Arc::new(async_std::sync::Mutex::new(t))
}

pub fn format_duration(dur: std::time::Duration) -> String {
    let secs = dur.as_secs();
    let nanos = dur.subsec_nanos();
    if secs > 60 {
        let mins = secs / 60;
        let secs = secs - mins * 60;
        format!("{}m{}s", mins, secs)
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
