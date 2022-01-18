pub type Mutex<T> = async_std::sync::Arc<async_std::sync::Mutex<T>>;
pub type Guard<T> = async_std::sync::MutexGuardArc<T>;

pub fn new<T>(t: T) -> async_std::sync::Arc<async_std::sync::Mutex<T>> {
    async_std::sync::Arc::new(async_std::sync::Mutex::new(t))
}

pub fn clone<T>(m: &Mutex<T>) -> Mutex<T> {
    async_std::sync::Arc::clone(m)
}
