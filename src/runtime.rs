// SPDX-License-Identifier: MPL-2.0

//! Shared async runtime for all network operations.
//!
//! This module provides a single Tokio runtime that all network operations share,
//! avoiding the overhead of creating a new runtime for each request.

use once_cell::sync::Lazy;
use std::future::Future;
use tokio::runtime::Runtime;

/// Shared multi-threaded Tokio runtime for all async operations.
/// Using 2 worker threads is sufficient for I/O-bound network operations.
static RUNTIME: Lazy<Runtime> = Lazy::new(|| {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .thread_name("hangar-async")
        .build()
        .expect("failed to create async runtime")
});

/// Execute a future on the shared runtime, blocking until completion.
/// Use this from synchronous code that needs to call async functions.
pub fn block_on<F: Future>(future: F) -> F::Output {
    RUNTIME.block_on(future)
}

/// Spawn a future on the shared runtime without blocking.
/// Returns a JoinHandle that can be used to await the result.
pub fn spawn<F>(future: F) -> tokio::task::JoinHandle<F::Output>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    RUNTIME.spawn(future)
}

/// Get a handle to the shared runtime for more advanced use cases.
#[allow(dead_code)]
pub fn handle() -> tokio::runtime::Handle {
    RUNTIME.handle().clone()
}
