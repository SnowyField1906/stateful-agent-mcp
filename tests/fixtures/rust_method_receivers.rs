use crate::Store as Data;
struct Store(i32);
struct Queue;
trait Putter {
    fn put(&self);
}
impl Store {
    fn new() -> Self {
        Store(0)
    }
    fn put(&self) {}
    fn receiver_is_self(store: Self) {
        store.put();
    }
    fn built_with_self() {
        let store = Self::new();
        store.put();
    }
}
impl Queue {
    fn put(&self) {}
}
fn annotated(store: &Store) {
    store.put();
}
fn aliased(store: Data) {
    store.put();
}
fn wrapped(store: Box<Store>) {
    store.put();
}
fn typed_local() {
    let queue: Queue = Queue;
    queue.put();
}
fn unit_struct_value() {
    let queue = Queue;
    queue.put();
}
fn constructed() {
    let store = Store::new();
    store.put();
}
fn tuple_constructed() {
    let store = Store(1);
    store.put();
}
fn unknown(value: &dyn Putter) {
    value.put();
}
fn pair() -> (Queue, ()) {
    (Queue, ())
}
fn shadowed_pattern(store: &Store) {
    let (store, _) = pair();
    store.put();
}
fn match_shadow(store: &Store, maybe: Option<&Queue>) {
    match maybe {
        Some(store) => store.put(),
        None => store.put(),
    }
}
fn for_shadow(store: &Store, queues: &[Queue]) {
    for store in queues {
        store.put();
    }
}
fn if_let_shadow(store: &Store, maybe: Option<&Queue>) {
    if let Some(store) = maybe {
        store.put();
    }
}
fn while_let_shadow(store: &Store, maybe: Option<&Queue>) {
    while let Some(store) = maybe {
        store.put();
        break;
    }
}
