// =========================================
// =========================================
// crates/motionloom/src/common/gpu_async/native.rs

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Condvar, Mutex};
use std::task::{Context, Poll, Waker};
use std::thread;
use std::time::Duration;

/// Shared state guarded by a `Mutex`/`Condvar` pair so that the background
/// polling thread can sleep until new GPU work arrives without lost wakeups.
struct DevicePollerState {
    shutdown: bool,
    pending: usize,
}

/// Background thread that keeps a wgpu device polled so that asynchronous
/// callbacks (such as `Buffer::map_async`) complete without blocking the
/// calling task.
///
/// The thread sleeps on a condition variable when no GPU callbacks are pending
/// and only polls while work is in flight. A small sleep during active polling
/// prevents a tight busy loop. The thread exits cleanly when the owning
/// `DevicePoller` is dropped; clones share the poll state but do not keep the
/// thread alive.
pub struct DevicePoller {
    state: Arc<(Mutex<DevicePollerState>, Condvar)>,
    // Only the poller that spawned the thread holds the join handle. Clones
    // receive `None` so that shutdown is performed exactly once.
    handle: Option<thread::JoinHandle<()>>,
}

impl Clone for DevicePoller {
    fn clone(&self) -> Self {
        Self {
            state: self.state.clone(),
            handle: None,
        }
    }
}

impl DevicePoller {
    /// Spawn a background thread that polls `device` until the owning poller
    /// is dropped.
    pub fn start(device: Arc<wgpu::Device>) -> Self {
        let state = Arc::new((
            Mutex::new(DevicePollerState {
                shutdown: false,
                pending: 0,
            }),
            Condvar::new(),
        ));
        let state_for_thread = state.clone();
        let handle = thread::spawn(move || {
            let (lock, cvar) = &*state_for_thread;
            loop {
                let mut guard = lock.lock().expect("poller state lock");
                while !guard.shutdown && guard.pending == 0 {
                    guard = cvar.wait(guard).expect("poller cvar wait");
                }
                let should_exit = guard.shutdown;
                let has_work = guard.pending > 0;
                // Drop the lock while polling so request/poll_complete do not
                // block on the mutex held by this thread.
                drop(guard);
                if has_work {
                    // PollType::Poll is non-blocking; it processes callbacks
                    // for work that has already finished on the GPU queue.
                    device.poll(wgpu::PollType::Poll).ok();
                    thread::sleep(Duration::from_micros(100));
                }
                if should_exit {
                    break;
                }
            }
        });
        Self {
            state,
            handle: Some(handle),
        }
    }

    /// Notify the poller that a new asynchronous GPU operation has been
    /// submitted and must be polled to completion.
    pub fn request_poll(&self) {
        let (lock, cvar) = &*self.state;
        let mut guard = lock.lock().expect("poller state lock");
        guard.pending += 1;
        cvar.notify_one();
    }

    /// Notify the poller that a previously submitted asynchronous GPU
    /// operation has completed.
    pub fn poll_complete(&self) {
        let (lock, _cvar) = &*self.state;
        let mut guard = lock.lock().expect("poller state lock");
        let previous = guard.pending;
        guard.pending = guard.pending.saturating_sub(1);
        debug_assert!(
            previous > 0,
            "poll_complete called without a matching request_poll"
        );
    }
}

impl Drop for DevicePoller {
    fn drop(&mut self) {
        // Only the owning handle shuts down the background thread. Clones
        // share state but must not join the thread.
        if self.handle.is_none() {
            return;
        }

        let (lock, cvar) = &*self.state;
        let mut guard = lock.lock().expect("poller state lock");
        guard.shutdown = true;
        cvar.notify_one();
        drop(guard);

        if let Some(handle) = self.handle.take() {
            handle.join().ok();
        }
    }
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
    /// when the mapping is ready to read. The supplied `poller` is woken so
    /// that the GPU callback can fire on a background thread.
    pub fn new(poller: &DevicePoller, buffer: &wgpu::Buffer) -> Self {
        let state = Arc::new(MapAsyncState::new());
        let state_clone = state.clone();
        let poller = poller.clone();
        poller.request_poll();
        buffer
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                state_clone.set_result(result);
                poller.poll_complete();
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
