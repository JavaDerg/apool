# apool
This crate allows you to create a Pool of any type `T: sSend + Sync` lazily.
When trying to acquire a `&mut T`
- if no instances have been created yet a new one will be created,
- if all instances are in use, a new instance will be created,
- unless a specified pool maximum is reached the task will wait for a instance to become free again.

`T` does not need to implement Clone, instead the Pool requites a type `S` which acts as State, and a `dyn Fn(&mut S, &mut PoolTransformer<T>)` which can create new instances of `T`.
The `Fn` will only be called if the maximum has not yet been reached.

## Usage
```
let pool = Pool::<FakeDatabase, &'static str>::new(
    4,
    "fakedb://127.0.0.1:1337/fake_website",
    |state, transformer| {
        let url = *state;
        transformer.spawn(async {
            FakeDatabase::connect(url).await.unwrap()
        });
    },
);

let guard = pool.get().await;
guard.commit_data().await;
```