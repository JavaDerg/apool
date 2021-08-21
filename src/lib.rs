mod sync;
#[cfg(test)]
mod tests;

use smallvec::SmallVec;
use std::collections::VecDeque;
use std::future::Future;
use std::sync::Arc;
use sync::*;
use std::ops::{Deref, DerefMut};

pub struct Pool<T: Sync, S: Sync> {
    state: Mutex<(usize, S)>,
    pool: Mutex<VecDeque<PoolEntry<T>>>,
    used: Semaphore,
    max: usize,
    transformer: Box<dyn Fn(&mut S, &mut PoolTransformer<T>)>,
}

pub struct PoolGuard<'a, T: Sync, S: Sync> {
    pool: &'a Pool<T, S>,
    id: usize,
    inner: OwnedMutexGuard<T>,
    _permit: SemaphorePermit<'a>,
}

pub struct PoolTransformer<T: Sync> {
    spawn: SmallVec<[Box<dyn Future<Output = T> + Send + Unpin>; 4]>,
}

impl<T: Sync, S: Sync> Pool<T, S> {
    pub fn new(
        max: usize,
        state: S,
        transform: impl Fn(&mut S, &mut PoolTransformer<T>) + Sync + 'static,
    ) -> Self {
        if max == 0 {
            panic!("max pool size is not allowed to be 0");
        }
        Self {
            state: Mutex::new((0, state)),
            pool: Mutex::new(VecDeque::with_capacity(max)),
            used: Semaphore::new(0),
            max,
            transformer: Box::new(transform),
        }
    }

    pub async fn get(&self) -> PoolGuard<'_, T, S> {
        let permit = match self.used.try_acquire() {
            Ok(permit) => permit,
            Err(_) => {
                self.try_spawn_new().await;
                self.used
                    .acquire()
                    .await
                    .expect("Semaphore should not be closed")
            }
        };
        let mut llock = self.pool.lock().await;
        let entry = match llock.pop_back() {
            Some(entry) => entry,
            None => panic!("Obtaining a pool entry should not fail"),
        };

        let PoolEntry { id, mutex } = entry.clone();
        llock.push_front(entry);

        PoolGuard {
            pool: self,
            inner: match mutex.try_lock_owned() {
                Ok(lock) => lock,
                Err(_) => panic!("Invalid pool list order"),
            },
            id,
            _permit: permit,
        }
    }

    async fn try_spawn_new(&self) {
        if { self.pool.lock().await.len() } >= self.max {
            return;
        }
        if let Ok(mut guard) = self.state.try_lock() {
            let (id, state) = &mut *guard;
            let mut transformer = PoolTransformer {
                spawn: SmallVec::new(),
            };
            (self.transformer)(state, &mut transformer);
            let mut llock = self.pool.lock().await;
            let len = transformer.spawn.len();
            for item in transformer.spawn {
                let item = item.await;
                *id += 1;
                llock.push_back(PoolEntry {
                    id: *id,
                    mutex: Arc::new(Mutex::new(item)),
                });
            }
            drop(llock);
            self.used.add_permits(len);
        }
    }
}

struct PoolEntry<T: Sync> {
    id: usize,
    mutex: Arc<Mutex<T>>,
}

impl<'a, T: Sync, S: Sync> Deref for PoolGuard<'a, T, S> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &*self.inner
    }
}

impl<'a, T: Sync, S: Sync> DerefMut for PoolGuard<'a, T, S> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut *self.inner
    }
}

impl<'a, T: Sync, S: Sync> Drop for PoolGuard<'a, T, S> {
    fn drop(&mut self) {
        tokio::runtime::Handle::current().block_on(async {
            let mut llock = self.pool.pool.lock().await;
            let index = (0usize..)
                .zip(llock.iter())
                .find(|item| item.1.id == self.id)
                .map(|item| item.0)
                .unwrap();
            let item = llock.remove(index).unwrap();
            llock.push_front(item);
        });
    }
}

impl<T: Sync> Clone for PoolEntry<T> {
    fn clone(&self) -> Self {
        Self {
            id: self.id,
            mutex: self.mutex.clone(),
        }
    }
}