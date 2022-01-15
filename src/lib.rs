use std::io;

pub mod args;
pub mod config;

pub fn other(desc: &str) -> io::Error {
    io::Error::new(io::ErrorKind::Other, desc)
}
