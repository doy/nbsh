pub type Mutex<T> = async_std::sync::Arc<async_std::sync::Mutex<T>>;

pub fn new<T>(t: T) -> async_std::sync::Arc<async_std::sync::Mutex<T>> {
    async_std::sync::Arc::new(async_std::sync::Mutex::new(t))
}

pub fn unwrap<T: std::fmt::Debug>(t: Mutex<T>) -> Option<T> {
    if let Ok(mutex) = async_std::sync::Arc::try_unwrap(t) {
        Some(async_std::sync::Mutex::into_inner(mutex))
    } else {
        None
    }
}
