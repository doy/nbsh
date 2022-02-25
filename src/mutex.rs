pub type Mutex<T> = std::sync::Arc<tokio::sync::Mutex<T>>;
pub type Guard<T> = tokio::sync::OwnedMutexGuard<T>;

pub fn new<T>(t: T) -> Mutex<T> {
    std::sync::Arc::new(tokio::sync::Mutex::new(t))
}

pub fn clone<T>(m: &Mutex<T>) -> Mutex<T> {
    std::sync::Arc::clone(m)
}
