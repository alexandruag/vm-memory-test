pub mod data_init;
pub mod errno;
pub mod guest_address;
pub mod guest_memory;
pub mod mmap;
pub mod shm;
pub mod volatile_memory;

pub use data_init::DataInit;
pub use errno::{errno_result, Error, Result};
pub use guest_address::GuestAddress;
pub use guest_memory::GuestMemory;
pub use mmap::MemoryMapping;
pub use volatile_memory::{VolatileMemory, VolatileMemoryError};

use libc::{sysconf, _SC_PAGESIZE};

/// Safe wrapper for `sysconf(_SC_PAGESIZE)`.
#[inline(always)]
pub fn pagesize() -> usize {
    // Trivially safe
    unsafe { sysconf(_SC_PAGESIZE) as usize }
}
