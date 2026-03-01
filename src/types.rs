use std::sync::RwLock;

pub(crate) type Nick = RwLock<Option<String>>;
