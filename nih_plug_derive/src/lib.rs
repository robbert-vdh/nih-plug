extern crate proc_macro;

use proc_macro::TokenStream;
use quote::quote;
use syn::spanned::Spanned;

#[proc_macro_derive(Params, attributes(id))]
pub fn derive_params(input: TokenStream) -> TokenStream {
    let ast = syn::parse_macro_input!(input as syn::DeriveInput);

    let struct_name = &ast.ident;
    let fields = match ast.data {
        syn::Data::Struct(syn::DataStruct {
            fields: syn::Fields::Named(named_fields),
            ..
        }) => named_fields,
        _ => {
            return syn::Error::new(
                ast.span(),
                "Deriving Params is only supported on structs with named fields",
            )
            .to_compile_error()
            .into()
        }
    };

    // We only care about fields with an `id` attribute. We'll build a that creates a hashmap
    // containing pointers to those parmaeters.
    let mut param_insert_tokens = Vec::new();
    for field in fields.named {
        let field_name = match &field.ident {
            Some(ident) => ident,
            _ => continue,
        };

        // We'll add another attribute for persistent fields later, and that's going to be mutually
        // exclusive with this id attribute
        let mut id_attr = None;
        for attr in field.attrs {
            if attr.path.is_ident("id") {
                match attr.parse_meta() {
                    Ok(syn::Meta::NameValue(syn::MetaNameValue {
                        lit: syn::Lit::Str(s),
                        ..
                    })) => {
                        if id_attr.is_none() {
                            id_attr = Some(s.value());
                        } else {
                            return syn::Error::new(attr.span(), "Duplicate id attribute")
                                .to_compile_error()
                                .into();
                        }
                    }
                    _ => {
                        return syn::Error::new(
                            attr.span(),
                            "The id attribute should be a key-value pair with a string argument: #[id = \"foo_bar\"]",
                        )
                        .to_compile_error()
                        .into()
                    }
                };
            }
        }

        if let Some(param_id) = id_attr {
            // The specific parameter types know how to convert themselves into the correct ParamPtr
            // variant
            param_insert_tokens
                .push(quote! { param_map.insert(#param_id, self.#field_name.as_ptr()); });
        }
    }

    quote! {
        impl Params for #struct_name {
            fn param_map(
                self: std::pin::Pin<&Self>,
            ) -> std::collections::HashMap<&'static str, nih_plug::params::ParamPtr> {
                let mut param_map = std::collections::HashMap::new();

                #(#param_insert_tokens)*

                param_map
            }
        }
    }
    .into()
}
