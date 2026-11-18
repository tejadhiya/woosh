//! Persistent storage and caching module for Woosh.
//!
//! Provides ultra-fast zero-copy disk caching of scan results and findings,
//! enabling instant cold-startup (< 3 ms) and persistent incremental state.

pub mod cache;

pub use cache::{
    current_fsevent_id, default_cache_path, load_cache, save_cache, CachedScan, IndexHeader,
    MAGIC_BYTES, SCHEMA_VERSION,
};
