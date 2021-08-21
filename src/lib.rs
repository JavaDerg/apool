mod sync;
#[cfg(test)]
mod tests;

use smallvec::SmallVec;
use std::collections::VecDeque;
use std::future::Future;
use std::sync::Arc;
use sync::*;
use std::ops::{Deref, DerefMut};
use std::pin::Pin;

pub struct Pool<T: 'static + Sync + Send, S: Sync> {
    state: Mutex<(usize, S)>,
    pool: Arc<Mutex<VecDeque<PoolEntry<T>>>>,
    used: Arc<Semaphore>,
    max: usize,
    transformer: Box<dyn Fn(&mut S, &mut PoolTransformer<T>)>,
}

pub struct PoolGuard<'a, T: 'static + Sync + Send, S: Sync> {
    pool: &'a Pool<T, S>,
    id: usize,
    inner: OwnedMutexGuard<T>,
    permit: Option<OwnedSemaphorePermit>,
}

pub struct PoolTransformer<'a, T: 'static + Sync + Send> {
    spawn: SmallVec<[Pin<Box<dyn Future<Output = T> + 'a>>; 4]>,
}

struct PoolEntry<T: 'static + Sync + Send> {
    id: usize,
    mutex: Arc<Mutex<T>>,
}

impl<T: 'static + Sync + Send, S: Sync> Pool<T, S> {
    pub fn new<'a>(
        max: usize,
        state: S,
        transform: impl Fn(&mut S, &mut PoolTransformer<T>) + Sync + 'static,
    ) -> Self {
        if max == 0 {
            panic!("max pool size is not allowed to be 0");
        }
        Self {
            state: Mutex::new((0, state)),
            pool: Arc::new(Mutex::new(VecDeque::with_capacity(max))),
            used: Arc::new(Semaphore::new(0)),
            max,
            transformer: Box::new(transform),
        }
    }

    pub async fn get(&self) -> PoolGuard<'_, T, S> {
        let permit = match self.used.clone().try_acquire_owned() {
            Ok(permit) => permit,
            Err(_) => {
                self.try_spawn_new().await;
                self.used.clone()
                    .acquire_owned()
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
            permit: Some(permit),
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

impl<'a, T: 'static + Sync + Send> PoolTransformer<'a, T> {
    pub fn spawn(&mut self, future: impl Future<Output = T> + 'a) {
        self.spawn.push(Box::pin(future));
    }
}

impl<'a, T: 'static + Sync + Send, S: Sync> Deref for PoolGuard<'a, T, S> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &*self.inner
    }
}

impl<'a, T: 'static + Sync + Send, S: Sync> DerefMut for PoolGuard<'a, T, S> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut *self.inner
    }
}

impl<'a, T: 'static + Sync + Send, S: Sync> Drop for PoolGuard<'a, T, S> {
    fn drop(&mut self) {
        let permit = self.permit.take().unwrap();
        let mutex = self.pool.pool.clone();
        let id = self.id;

        tokio::runtime::Handle::current().spawn(async move {
            let mut llock = mutex.lock().await;
            let index = (0usize..)
                .zip(llock.iter())
                .find(|item| item.1.id == id)
                .map(|item| item.0)
                .unwrap();
            let item = llock.remove(index).unwrap();
            llock.push_back(item);
            drop(permit);
        });
    }
}

impl<T: 'static + Sync + Send> Clone for PoolEntry<T> {
    fn clone(&self) -> Self {
        Self {
            id: self.id,
            mutex: self.mutex.clone(),
        }
    }
}