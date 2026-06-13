//! Project prelude: the single point that controls the default map hasher.
pub type LibDefaultHasher = rustc_hash::FxBuildHasher;
pub type HashMap<K, V> = std::collections::HashMap<K, V, LibDefaultHasher>;
pub type HashSet<T> = std::collections::HashSet<T, LibDefaultHasher>;
