#![cfg_attr(not(test), no_main)]
// without std there is no panic formatting, panics trap, see `panic` below
#![cfg_attr(target_arch = "wasm32", no_std)]

extern crate alloc;

use crate::{
    componentized::constants::config_values::values,
    exports::wasi::config::store::{Error, Guest},
};
use alloc::{string::String, vec::Vec};

#[cfg(target_arch = "wasm32")]
mod bump;

#[cfg(target_arch = "wasm32")]
#[global_allocator]
static ALLOCATOR: bump::BumpAllocator = bump::BumpAllocator::new();

/// Traps without formatting a message, nothing here is expected to panic.
#[cfg(target_arch = "wasm32")]
#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    core::arch::wasm32::unreachable()
}

pub(crate) struct Config;

impl Guest for Config {
    /// The value of the last entry named `key`, as with the WIT `map` type,
    /// later entries with the same key replace earlier ones.
    fn get(key: String) -> Result<Option<String>, Error> {
        Ok(values()
            .into_iter()
            .rev()
            .find_map(|(k, v)| (k == key).then_some(v)))
    }

    fn get_all() -> Result<Vec<(String, String)>, Error> {
        Ok(values())
    }
}

wit_bindgen::generate!({
    path: "../wit",
    world: "config",
    generate_all
});

export!(Config);
