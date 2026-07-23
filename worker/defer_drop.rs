/// Wraps a `T` so that it is always dropped on a freshly spawned OS thread.
///
/// Use this when `T` contains an internal `tokio::runtime::Runtime` (e.g.
/// `reqwest::blocking::Client`) and may be dropped from within a tokio async or
/// blocking-pool thread, where dropping a nested runtime would panic.
pub(crate) struct DeferDrop<T: Send + 'static>(Option<T>);

impl<T: Send + 'static> DeferDrop<T> {
    pub(crate) fn new(val: T) -> Self {
        Self(Some(val))
    }
}

impl<T: Send + 'static> std::ops::Deref for DeferDrop<T> {
    type Target = T;
    fn deref(&self) -> &T {
        self.0.as_ref().unwrap()
    }
}

impl<T: Send + 'static> Drop for DeferDrop<T> {
    fn drop(&mut self) {
        if let Some(val) = self.0.take() {
            std::thread::spawn(move || drop(val));
        }
    }
}
