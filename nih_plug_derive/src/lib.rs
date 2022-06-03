use proc_macro::TokenStream;

mod enums;
mod params;

/// Derive the `Enum` trait for simple enum parameters. See `EnumParam` for more information.
#[proc_macro_derive(Enum, attributes(name, id))]
pub fn derive_enum(input: TokenStream) -> TokenStream {
    enums::derive_enum(input)
}

/// Derive the `Params` trait for your plugin's parameters struct. See the `Plugin` trait.
#[proc_macro_derive(Params, attributes(id, persist, nested))]
pub fn derive_params(input: TokenStream) -> TokenStream {
    params::derive_params(input)
}
