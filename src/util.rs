pub type Mutex<T> = async_std::sync::Arc<async_std::sync::Mutex<T>>;

pub fn mutex<T>(t: T) -> Mutex<T> {
    async_std::sync::Arc::new(async_std::sync::Mutex::new(t))
}
