#![cfg_attr(not(test), no_main)]

use crate::{
    componentized::component::types::{ErrorCode, Wasm},
    exports::componentized::constants::factory::{Guest, Overrides, WitSource},
};
use componentized_constants::{create_component, decode_world, parse_world};

mod parsed;

pub(crate) struct Factory;

impl Guest for Factory {
    #[allow(async_fn_in_trait)]
    async fn create(
        wit: WitSource,
        world: Option<String>,
        overrides: Option<Overrides>,
    ) -> Result<Wasm, ErrorCode> {
        let (resolve, world) = match wit {
            WitSource::Wit(text) => parse_world(&text, world.as_deref()),
            WitSource::Wasm(bytes) => decode_world(&bytes, world.as_deref()),
            WitSource::Parsed(wit) => parsed_world(wit, world.as_deref()),
        }
        .map_err(to_error)?;
        let overrides = overrides.as_ref().map(|overrides| match overrides {
            Overrides::Wave(src) => componentized_constants::Overrides::Wave(src),
        });
        create_component(&resolve, world, overrides).map_err(to_error)
    }
}

/// Implements the world named `world`, or when absent, the world of the
/// component the WIT was extracted from.
fn parsed_world(
    wit: parsed::Wit,
    world: Option<&str>,
) -> anyhow::Result<(wit_parser::Resolve, wit_parser::WorldId)> {
    let parsed = parsed::to_resolve(wit)?;
    let world = match (world, parsed.component_world) {
        (None, Some(world)) => world,
        (world, _) => parsed.resolve.select_world(&[parsed.package], world)?,
    };
    Ok((parsed.resolve, world))
}

fn to_error(err: anyhow::Error) -> ErrorCode {
    ErrorCode::Other(Some(format!("{err:#}")))
}

wit_bindgen::generate!({
    path: "../wit",
    world: "factory",
    generate_all
});

export!(Factory);
