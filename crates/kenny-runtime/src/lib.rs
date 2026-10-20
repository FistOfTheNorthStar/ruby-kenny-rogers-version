//! Kenny runtime: the object model, method dispatch and core library that
//! ahead-of-time compiled Ruby programs link against.

#![allow(dangerous_implicit_autorefs, clippy::missing_safety_doc)]

#[global_allocator]
static ALLOC: gc::CountingAlloc = gc::CountingAlloc;

pub mod abi;
pub mod backtrace;
pub mod array;
pub mod bigdecimal;
pub mod boot;
pub mod capi;
pub mod cmap;
pub mod cgi_escape;
pub mod compat;
pub mod core;
pub mod date;
pub mod digest;
pub mod fmt;
pub mod gc;
pub mod hash;
pub mod io;
pub mod json;
pub mod kernel;
pub mod khash;
pub mod lazy;
pub mod marshal;
pub mod methodobj;
pub mod misc;
pub mod mt;
pub mod extra;
pub mod nokogiri;
pub mod numeric;
pub mod opt;
pub mod pack;
pub mod openssl;
pub mod rational;
pub mod regex;
pub mod server;
pub mod socket;
pub mod sqlite;
pub mod string;
pub mod strscan;
pub mod svec;
pub mod thread;
pub mod vfs;
pub mod visibility;
pub mod yaml;
pub mod zlib;
