// src/engine/mod.rs
pub mod llm;
pub mod router;
pub mod thread_pool;
pub mod vision;
pub mod smart_extract; // Phase 4: Added smart entities module

pub use router::SystemRouter;
pub use thread_pool::ThreadPool;