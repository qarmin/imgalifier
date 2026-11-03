//! The handful of C type aliases and errno values rav1d needs, so the `libc` crate is not required.
#![allow(non_camel_case_types)]

pub type ptrdiff_t = isize;
pub type intptr_t = isize;
pub type uintptr_t = usize;
pub type off_t = i64;

pub const ENOENT: i32 = 2;
pub const EAGAIN: i32 = 11;
pub const ENOMEM: i32 = 12;
pub const EINVAL: i32 = 22;
pub const ERANGE: i32 = 34;
pub const ENOPROTOOPT: i32 = 92;
