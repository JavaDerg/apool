#[cfg(feature = "tokio")]
pub type Mutex<T> = tokio::sync::Mutex<T>;

#[cfg(feature = "tokio")]
pub type OwnedMutexGuard<T> = tokio::sync::OwnedMutexGuard<T>;

#[cfg(feature = "tokio")]
pub type Semaphore = tokio::sync::Semaphore;

#[cfg(feature = "tokio")]
pub type SemaphorePermit<'a> = tokio::sync::SemaphorePermit<'a>;
