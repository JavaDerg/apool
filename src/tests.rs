use crate::Pool;

#[test]
#[should_panic]
fn zero_size_disallowed() {
    let _ = Pool::<(), ()>::new(0, (), |_, _| ());
}

#[test]
fn basic_usize_test() {
    let pool = Pool::<usize, usize>::new(4, 0, |state, transform| {
        let sc = *state;
        transform.spawn(async move { sc });
        *state += 1;
    });
    tokio::runtime::Builder::new_current_thread().build().unwrap().block_on(async {
        let guard1 = pool.get().await;
        assert_eq!(*guard1, 0);
        let guard2 = pool.get().await;
        assert_eq!(*guard2, 1);
        let guard3 = pool.get().await;
        assert_eq!(*guard3, 2);
        let guard4 = pool.get().await;
        assert_eq!(*guard4, 3);

        drop(guard3);
        drop(guard2);

        let guard2 = pool.get().await;
        assert_eq!(*guard2, 1);
        let guard3 = pool.get().await;
        assert_eq!(*guard3, 2);
    });
}
