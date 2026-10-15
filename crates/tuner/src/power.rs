//! macOS sleep/wake notifications (`IORegisterForSystemPower`).
//!
//! A dedicated thread runs the CFRunLoop. Before sleep it sends `WillSleep` to the main
//! loop and waits for the acknowledgement (up to 5 s) before allowing sleep: that way the
//! fans go back to Auto before the machine sleeps.

use std::sync::mpsc::{Receiver, SyncSender};

pub enum PowerEvent {
    /// The system is going to sleep. Reply on `ack` once the fans are back to Auto.
    WillSleep { ack: SyncSender<()> },
    /// The system woke up.
    Woke,
}

/// Starts listening. Outside macOS returns a channel that never receives anything.
pub fn start() -> Result<Receiver<PowerEvent>, String> {
    imp::start()
}

#[cfg(not(any(target_os = "macos", feature = "check-ffi")))]
mod imp {
    use super::PowerEvent;
    use std::sync::mpsc::{channel, Receiver};

    pub fn start() -> Result<Receiver<PowerEvent>, String> {
        let (tx, rx) = channel();
        // Keep the sending side alive so the channel does not close.
        std::mem::forget(tx);
        Ok(rx)
    }
}

#[cfg(any(target_os = "macos", feature = "check-ffi"))]
mod imp {
    use super::PowerEvent;
    use std::ffi::c_void;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::mpsc::{channel, sync_channel, Receiver, Sender};
    use std::sync::{Mutex, OnceLock};
    use std::time::Duration;

    type IoConnect = u32;
    type IoObject = u32;
    type IONotificationPortRef = *mut c_void;
    type CFRunLoopRef = *mut c_void;
    type CFRunLoopSourceRef = *mut c_void;
    type CFStringRef = *const c_void;
    type Callback = extern "C" fn(
        refcon: *mut c_void,
        service: IoObject,
        message_type: u32,
        argument: *mut c_void,
    );

    const MSG_CAN_SYSTEM_SLEEP: u32 = 0xE000_0270;
    const MSG_SYSTEM_WILL_SLEEP: u32 = 0xE000_0280;
    const MSG_SYSTEM_HAS_POWERED_ON: u32 = 0xE000_0300;

    #[cfg_attr(target_os = "macos", link(name = "IOKit", kind = "framework"))]
    extern "C" {
        fn IORegisterForSystemPower(
            refcon: *mut c_void,
            port: *mut IONotificationPortRef,
            callback: Callback,
            notifier: *mut IoObject,
        ) -> IoConnect;
        fn IONotificationPortGetRunLoopSource(port: IONotificationPortRef) -> CFRunLoopSourceRef;
        fn IOAllowPowerChange(kernel_port: IoConnect, notification_id: isize) -> i32;
    }

    #[cfg_attr(target_os = "macos", link(name = "CoreFoundation", kind = "framework"))]
    extern "C" {
        fn CFRunLoopGetCurrent() -> CFRunLoopRef;
        fn CFRunLoopAddSource(rl: CFRunLoopRef, source: CFRunLoopSourceRef, mode: CFStringRef);
        fn CFRunLoopRun();
        static kCFRunLoopDefaultMode: CFStringRef;
    }

    static ROOT_PORT: AtomicU32 = AtomicU32::new(0);
    static SENDER: OnceLock<Mutex<Sender<PowerEvent>>> = OnceLock::new();

    fn send(ev: PowerEvent) -> bool {
        SENDER
            .get()
            .and_then(|m| m.lock().ok())
            .map(|tx| tx.send(ev).is_ok())
            .unwrap_or(false)
    }

    extern "C" fn on_power(
        _refcon: *mut c_void,
        _service: IoObject,
        message_type: u32,
        argument: *mut c_void,
    ) {
        let port = ROOT_PORT.load(Ordering::SeqCst);
        match message_type {
            MSG_CAN_SYSTEM_SLEEP => {
                // SAFETY: argument received from the kernel itself for this notification.
                unsafe { IOAllowPowerChange(port, argument as isize) };
            }
            MSG_SYSTEM_WILL_SLEEP => {
                let (ack, done) = sync_channel(1);
                if send(PowerEvent::WillSleep { ack }) {
                    let _ = done.recv_timeout(Duration::from_secs(5));
                }
                // SAFETY: same; we always allow sleep, even without acknowledgement.
                unsafe { IOAllowPowerChange(port, argument as isize) };
            }
            MSG_SYSTEM_HAS_POWERED_ON => {
                send(PowerEvent::Woke);
            }
            _ => {}
        }
    }

    pub fn start() -> Result<Receiver<PowerEvent>, String> {
        let (tx, rx) = channel();
        SENDER
            .set(Mutex::new(tx))
            .map_err(|_| "power::start called twice".to_string())?;
        let (ready_tx, ready_rx) = channel::<Result<(), String>>();
        std::thread::Builder::new()
            .name("power".into())
            .spawn(move || {
                let mut port: IONotificationPortRef = std::ptr::null_mut();
                let mut notifier: IoObject = 0;
                // SAFETY: documented IOKit API; pointers valid during the call.
                let root = unsafe {
                    IORegisterForSystemPower(
                        std::ptr::null_mut(),
                        &mut port,
                        on_power,
                        &mut notifier,
                    )
                };
                if root == 0 || port.is_null() {
                    let _ = ready_tx.send(Err("IORegisterForSystemPower failed".into()));
                    return;
                }
                ROOT_PORT.store(root, Ordering::SeqCst);
                // SAFETY: the source belongs to the port, which lives as long as the thread.
                unsafe {
                    let source = IONotificationPortGetRunLoopSource(port);
                    CFRunLoopAddSource(CFRunLoopGetCurrent(), source, kCFRunLoopDefaultMode);
                }
                let _ = ready_tx.send(Ok(()));
                // SAFETY: runs this thread's run loop forever.
                unsafe { CFRunLoopRun() };
            })
            .map_err(|e| format!("could not create the power thread: {e}"))?;
        ready_rx
            .recv_timeout(Duration::from_secs(5))
            .map_err(|_| "the power thread did not respond".to_string())??;
        Ok(rx)
    }
}
