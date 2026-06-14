// =========================================
// =========================================
// crates/motionloom/src/common/gpu_async/wasm.rs

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};

/// WebAssembly stub for the native background polling thread.
///
/// The browser wgpu backend schedules its own callbacks via JavaScript
/// promises, so no separate polling thread is required. This type keeps the
/// same public API so that platform-agnostic code can clone and pass it around
/// without `cfg` branches.
#[derive(Clone)]
pub struct DevicePoller;

impl DevicePoller {
    /// Returns a no-op poller. The `device` argument is kept for API symmetry
    /// with the native implementation.
    pub fn start(_device: Arc<wgpu::Device>) -> Self {
        Self
    }

    /// No-op: the browser runtime drives the GPU callbacks.
    pub fn request_poll(&self) {}

    /// No-op: the browser runtime drives the GPU callbacks.
    pub fn poll_complete(&self) {}
}

/// Shared state that bridges a wgpu `map_async` callback with a `Future`.
struct MapAsyncState {
    result: Mutex<Option<Result<(), wgpu::BufferAsyncError>>>,
    waker: Mutex<Option<Waker>>,
}

impl MapAsyncState {
    fn new() -> Self {
        Self {
            result: Mutex::new(None),
            waker: Mutex::new(None),
        }
    }

    fn set_result(&self, result: Result<(), wgpu::BufferAsyncError>) {
        *self.result.lock().expect("map_async state result lock") = Some(result);
        if let Some(waker) = self
            .waker
            .lock()
            .expect("map_async state waker lock")
            .take()
        {
            waker.wake();
        }
    }

    fn poll(&self, cx: &mut Context<'_>) -> Poll<Result<(), wgpu::BufferAsyncError>> {
        let mut result = self.result.lock().expect("map_async state result lock");
        if let Some(res) = result.take() {
            return Poll::Ready(res);
        }
        *self.waker.lock().expect("map_async state waker lock") = Some(cx.waker().clone());
        Poll::Pending
    }
}

/// Future that resolves when `buffer.slice(..).map_async(Read, ...)` completes.
pub struct BufferMapAsyncFuture {
    state: Arc<MapAsyncState>,
}

impl BufferMapAsyncFuture {
    /// Start an asynchronous map of `buffer` and return a future that resolves
    /// when the mapping is ready to read.
    pub fn new(_poller: &DevicePoller, buffer: &wgpu::Buffer) -> Self {
        let state = Arc::new(MapAsyncState::new());
        let state_clone = state.clone();
        buffer
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                state_clone.set_result(result);
            });
        Self { state }
    }
}

impl Future for BufferMapAsyncFuture {
    type Output = Result<(), wgpu::BufferAsyncError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.state.poll(cx)
    }
}

/// Asynchronously request a wgpu adapter.
pub async fn request_adapter_async(
    instance: &wgpu::Instance,
    options: &wgpu::RequestAdapterOptions<'_, '_>,
) -> Result<wgpu::Adapter, wgpu::RequestAdapterError> {
    instance.request_adapter(options).await
}

/// Asynchronously request a wgpu device and queue.
pub async fn request_device_async(
    adapter: &wgpu::Adapter,
    descriptor: &wgpu::DeviceDescriptor<'_>,
) -> Result<(wgpu::Device, wgpu::Queue), wgpu::RequestDeviceError> {
    adapter.request_device(descriptor).await
}
