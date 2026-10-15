//! Minimal bridge to the Objective-C runtime, with no crates.
//!
//! On x86_64, `objc_msgSend` works for any signature whose return is a
//! pointer/integer/void or double; each function below casts it to the right signature.

#![allow(clippy::missing_safety_doc)]

use std::ffi::{c_void, CStr, CString};
use std::os::raw::c_char;

pub type Id = *mut c_void;
pub type Sel = *const c_void;
pub const NIL: Id = std::ptr::null_mut();
/// Objective-C `BOOL` on x86_64 is `signed char`.
pub type Bool = i8;

#[cfg_attr(target_os = "macos", link(name = "objc"))]
extern "C" {
    fn objc_getClass(name: *const c_char) -> Id;
    fn sel_registerName(name: *const c_char) -> Sel;
    fn objc_msgSend();
    fn objc_allocateClassPair(superclass: Id, name: *const c_char, extra: usize) -> Id;
    fn objc_registerClassPair(cls: Id);
    fn class_addMethod(cls: Id, name: Sel, imp: *const c_void, types: *const c_char) -> Bool;
}

// Ensures linking with AppKit and Foundation (the NS* classes come from there).
#[cfg_attr(target_os = "macos", link(name = "AppKit", kind = "framework"))]
#[cfg_attr(target_os = "macos", link(name = "Foundation", kind = "framework"))]
extern "C" {}

pub fn class(name: &CStr) -> Id {
    // SAFETY: valid C string; returns nil if the class does not exist.
    unsafe { objc_getClass(name.as_ptr()) }
}

pub fn sel(name: &CStr) -> Sel {
    // SAFETY: valid C string; selectors are registered and never freed.
    unsafe { sel_registerName(name.as_ptr()) }
}

fn msg_send_ptr() -> *const c_void {
    objc_msgSend as *const c_void
}

macro_rules! send_fn {
    ($name:ident ( $($arg:ident : $ty:ty),* ) -> $ret:ty) => {
        pub unsafe fn $name(obj: Id, s: &CStr $(, $arg: $ty)*) -> $ret {
            let f: unsafe extern "C" fn(Id, Sel $(, $ty)*) -> $ret = std::mem::transmute(msg_send_ptr());
            f(obj, sel(s) $(, $arg)*)
        }
    };
}

send_fn!(send0() -> Id);
send_fn!(send_id(a: Id) -> Id);
send_fn!(send_ptr(a: *const c_char) -> Id);
send_fn!(send_f64(a: f64) -> Id);
send_fn!(send_f64_f64(a: f64, b: f64) -> Id);
send_fn!(send_isize(a: isize) -> Id);
send_fn!(send_bool(a: Bool) -> Id);
send_fn!(send_ret_isize() -> isize);
send_fn!(send_id_sel_id(a: Id, b: Sel, c: Id) -> Id);
send_fn!(send_timer(interval: f64, target: Id, selector: Sel, info: Id, repeats: Bool) -> Id);

/// `NSString` autoreleased a partir de texto Rust.
pub fn nsstring(s: &str) -> Id {
    let c = CString::new(s.replace('\0', "")).unwrap_or_default();
    // SAFETY: stringWithUTF8String: copies the buffer, which lives until the call returns.
    unsafe { send_ptr(class(c"NSString"), c"stringWithUTF8String:", c.as_ptr()) }
}

pub type Imp = extern "C" fn(Id, Sel, Id);

/// Creates (once) an NSObject subclass with `-(void)name:(id)arg` methods.
pub fn define_class(name: &CStr, methods: &[(&CStr, Imp)]) -> Id {
    // SAFETY: runtime API; the class is registered exactly once, before any use.
    unsafe {
        let cls = objc_allocateClassPair(class(c"NSObject"), name.as_ptr(), 0);
        for (selector, imp) in methods {
            class_addMethod(cls, sel(selector), *imp as *const c_void, c"v@:@".as_ptr());
        }
        objc_registerClassPair(cls);
        cls
    }
}

/// `[[cls alloc] init]`.
pub fn new(cls: Id) -> Id {
    // SAFETY: standard NSObject alloc/init.
    unsafe { send0(send0(cls, c"alloc"), c"init") }
}

/// Reads an `NSString` as a Rust `String` (via `-UTF8String`).
pub unsafe fn nsstring_to_string(s: Id) -> Option<String> {
    if s.is_null() {
        return None;
    }
    let p = send0(s, c"UTF8String") as *const c_char;
    if p.is_null() {
        return None;
    }
    CStr::from_ptr(p).to_str().ok().map(|x| x.to_string())
}

/// Runs `f` inside its own NSAutoreleasePool.
pub fn with_pool<R>(f: impl FnOnce() -> R) -> R {
    let pool = new(class(c"NSAutoreleasePool"));
    let r = f();
    // SAFETY: the pool was created above on this same thread.
    unsafe { send0(pool, c"drain") };
    r
}
