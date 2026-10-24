//! Keeps the PC usable while a transcription runs: the process drops below
//! normal priority for the job, and the threads that must not wait for it
//! (the window, audio capture) raise themselves.

use windows::Win32::System::Threading::{
    GetCurrentProcess, GetCurrentThread, SetPriorityClass, SetThreadPriority, BELOW_NORMAL_PRIORITY_CLASS,
    NORMAL_PRIORITY_CLASS, THREAD_PRIORITY_HIGHEST,
};

/// Windows gives new threads (also ONNX Runtime's) the process's priority,
/// so it is the only handle on the model threads.
pub struct Background;

impl Background {
    pub fn new() -> Self {
        if let Err(error) = unsafe { SetPriorityClass(GetCurrentProcess(), BELOW_NORMAL_PRIORITY_CLASS) } {
            tracing::warn!(%error, "priority not lowered");
        }
        Self
    }
}

impl Drop for Background {
    fn drop(&mut self) {
        let _ = unsafe { SetPriorityClass(GetCurrentProcess(), NORMAL_PRIORITY_CLASS) };
    }
}

/// Two steps above the process: on par with other programs while a job runs,
/// ahead of the model threads.
pub fn raise_current_thread() {
    if let Err(error) = unsafe { SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_HIGHEST) } {
        tracing::warn!(%error, "thread priority not raised");
    }
}
