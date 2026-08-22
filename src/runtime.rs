use std::sync::{Mutex, MutexGuard, OnceLock};
use std::thread::JoinHandle;

use deno_core::{Extension, JsRuntime, RuntimeOptions};
use tokio::sync::mpsc;

/// Safety net only: the platform init below is the actual fix for deno_core#952. Held
/// across isolate construction and destruction, never while a live runtime is in use.
static ISOLATE_LIFECYCLE: Mutex<()> = Mutex::new(());

pub(crate) fn lifecycle_lock() -> MutexGuard<'static, ()> {
    ISOLATE_LIFECYCLE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Initializes V8 once per process, before any isolate exists. Snapshot builders must call
/// this too -- their isolates are bound by the same rule.
pub fn init_platform() {
    static ONCE: OnceLock<()> = OnceLock::new();
    ONCE.get_or_init(|| {
        // Memory Protection Keys (pkeys) are a CPU feature V8's default platform uses to
        // write-protect its own heap; it demands every V8-touching thread descend from the
        // one that called v8::Initialize(). Each Runtime owns a thread spawned from whichever
        // caller got there first, which violates that: V8 then segfaults inside
        // WasmCodePointerTable::AllocateUninitializedEntry(). The unprotected platform drops
        // pkeys, giving up one defense-in-depth layer against V8-internal memory-corruption
        // exploits -- accepted because mini runs local page JS, not untrusted multi-tenant JS.
        let platform = deno_core::v8::new_unprotected_default_platform(0, false).make_shared();
        JsRuntime::init_platform(Some(platform));
    });
}

/// Hand-built rather than via `extension!`: the macro buys nothing for a single fixed
/// extension and hides what is actually registered.
pub(crate) fn extension() -> Extension {
    Extension {
        name: "miniclient",
        ..Default::default()
    }
}

/// Everything Python asks of the isolate thread crosses as one of these. The ops of §4 land
/// here as further variants, each carrying the channel its answer goes back on.
enum Command {
    Close,
}

/// A V8 isolate plus the OS thread that exclusively owns it for its whole life.
pub struct Runtime {
    commands: mpsc::UnboundedSender<Command>,
    thread: Option<JoinHandle<()>>,
}

impl Runtime {
    pub fn new() -> Self {
        init_platform();
        let (commands, mut rx) = mpsc::unbounded_channel();
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let thread = std::thread::spawn(move || {
            let tokio = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("failed to build the isolate thread's tokio runtime");
            tokio.block_on(async move {
                let js = {
                    let _lock = lifecycle_lock();
                    JsRuntime::new(RuntimeOptions {
                        extensions: vec![extension()],
                        ..Default::default()
                    })
                };
                ready_tx.send(()).ok();

                // One variant today, so the loop always breaks on its first pass; §4's ops
                // add variants that handle a command and keep going.
                #[allow(clippy::never_loop)]
                while let Some(command) = rx.recv().await {
                    match command {
                        Command::Close => break,
                    }
                }
                let _lock = lifecycle_lock();
                drop(js);
            });
        });
        ready_rx
            .recv()
            .expect("isolate thread died while constructing the runtime");
        Self {
            commands,
            thread: Some(thread),
        }
    }

    /// Cannot return before the isolate is gone: the thread join is the happens-before.
    pub fn close(&mut self) {
        let Some(thread) = self.thread.take() else {
            return;
        };
        self.commands.send(Command::Close).ok();
        thread.join().expect("isolate thread panicked");
    }
}

impl Default for Runtime {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for Runtime {
    fn drop(&mut self) {
        self.close();
    }
}

#[cfg(test)]
mod tests {
    use super::Runtime;

    #[test]
    fn constructs_and_closes_concurrently() {
        let threads: Vec<_> = (0..8)
            .map(|_| {
                std::thread::spawn(|| {
                    for _ in 0..3 {
                        let mut runtime = Runtime::new();
                        runtime.close();
                    }
                    // The last one closes via Drop instead.
                    Runtime::new();
                })
            })
            .collect();
        for thread in threads {
            thread.join().unwrap();
        }
    }
}
