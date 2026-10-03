use std::thread::JoinHandle;

/// Handles remain resident after a semantic terminal event until native destructors exit.
/// The owning runtime serializes admission and cleanup under its state mutex.
#[derive(Default)]
pub(super) struct NativeThreads {
    closed: bool,
    quarantined: bool,
    handles: Vec<JoinHandle<()>>,
}
impl NativeThreads {
    pub fn can_spawn(&mut self, _capacity: usize) -> bool {
        self.reap();
        // Logical admission and retained pools cap native ownership. A handle may remain here
        // for a few instructions after it returned native state and sent its terminal event.
        !self.closed && !self.quarantined
    }
    pub fn retain(&mut self, handle: JoinHandle<()>) {
        self.handles.push(handle);
    }
    pub fn is_closed(&self) -> bool {
        self.closed || self.quarantined
    }
    pub fn close(&mut self) -> bool {
        self.closed = true;
        self.reap();
        !self.quarantined && self.handles.is_empty()
    }
    fn reap(&mut self) {
        let mut index = 0;
        while index < self.handles.len() {
            if self.handles[index].is_finished() {
                self.quarantined |= self.handles.swap_remove(index).join().is_err();
            } else {
                index += 1;
            }
        }
    }
}
