//! Real transport: user client of the `AppleSMC` IOKit service.
//! Reading works as a normal user; writing requires root.

use std::ffi::c_void;
use std::os::raw::c_char;

use crate::protocol::{KeyData, Transport, KERNEL_INDEX_SMC};
use crate::SmcError;

type KernReturn = i32;
type MachPort = u32;
type IoObject = MachPort;
type IoService = IoObject;
type IoConnect = IoObject;
type CFMutableDictionaryRef = *mut c_void;

const KERN_SUCCESS: KernReturn = 0;
/// `kIOMainPortDefault` / `kIOMasterPortDefault`.
const IO_MAIN_PORT_DEFAULT: MachPort = 0;

#[cfg_attr(target_os = "macos", link(name = "IOKit", kind = "framework"))]
extern "C" {
    fn IOServiceMatching(name: *const c_char) -> CFMutableDictionaryRef;
    fn IOServiceGetMatchingService(
        main_port: MachPort,
        matching: CFMutableDictionaryRef,
    ) -> IoService;
    fn IOServiceOpen(
        service: IoService,
        owning_task: MachPort,
        kind: u32,
        connect: *mut IoConnect,
    ) -> KernReturn;
    fn IOServiceClose(connect: IoConnect) -> KernReturn;
    fn IOObjectRelease(object: IoObject) -> KernReturn;
    fn IOConnectCallStructMethod(
        connection: MachPort,
        selector: u32,
        input: *const c_void,
        input_size: usize,
        output: *mut c_void,
        output_size: *mut usize,
    ) -> KernReturn;
}

extern "C" {
    /// What the `mach_task_self()` macro reads.
    static mach_task_self_: MachPort;
}

/// Open connection to the `AppleSMC`. Closes itself when it goes out of scope.
pub struct IoKitTransport {
    conn: IoConnect,
}

impl IoKitTransport {
    pub fn open() -> Result<Self, SmcError> {
        // SAFETY: documented IOKit calls; the matching dictionary is consumed by
        // IOServiceGetMatchingService, and the service is released right after opening.
        unsafe {
            let matching = IOServiceMatching(c"AppleSMC".as_ptr());
            if matching.is_null() {
                return Err(SmcError::NoSmc);
            }
            let service = IOServiceGetMatchingService(IO_MAIN_PORT_DEFAULT, matching);
            if service == 0 {
                return Err(SmcError::NoSmc);
            }
            let mut conn: IoConnect = 0;
            let kr = IOServiceOpen(service, mach_task_self_, 0, &mut conn);
            IOObjectRelease(service);
            if kr != KERN_SUCCESS {
                return Err(SmcError::Kernel {
                    call: "IOServiceOpen",
                    code: kr,
                });
            }
            Ok(IoKitTransport { conn })
        }
    }
}

impl Transport for IoKitTransport {
    fn call(&mut self, input: &KeyData) -> Result<KeyData, SmcError> {
        let mut output = KeyData::default();
        let mut out_size = std::mem::size_of::<KeyData>();
        // SAFETY: both structs are exactly 80 bytes (checked at compile time).
        let kr = unsafe {
            IOConnectCallStructMethod(
                self.conn,
                KERNEL_INDEX_SMC,
                input as *const KeyData as *const c_void,
                std::mem::size_of::<KeyData>(),
                &mut output as *mut KeyData as *mut c_void,
                &mut out_size,
            )
        };
        if kr != KERN_SUCCESS {
            return Err(SmcError::Kernel {
                call: "IOConnectCallStructMethod",
                code: kr,
            });
        }
        Ok(output)
    }
}

impl Drop for IoKitTransport {
    fn drop(&mut self) {
        // SAFETY: `conn` came from IOServiceOpen and is closed exactly once.
        unsafe {
            IOServiceClose(self.conn);
        }
    }
}
