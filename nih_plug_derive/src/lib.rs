extern crate proc_macro;

use proc_macro::TokenStream;
use quote::quote;
use syn::spanned::Spanned;

#[proc_macro_derive(Params, attributes(id, persist))]
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

    // We only care about fields with `id` and `persist` attributes. For the `id` fields we'll build
    // a mapping function that creates a hashmap containing pointers to those parmaeters. For the
    // `persist` function we'll create functions that serialize and deserialize those fields
    // individually (so they can be added and removed independently of eachother) using JSON.
    let mut param_mapping_insert_tokens = Vec::new();
    let mut field_serialize_tokens = Vec::new();
    let mut field_deserialize_tokens = Vec::new();
    for field in fields.named {
        let field_name = match &field.ident {
            Some(ident) => ident,
            _ => continue,
        };

        // These two attributes are mutually exclusive
        let mut id_attr: Option<String> = None;
        let mut persist_attr: Option<String> = None;
        for attr in &field.attrs {
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
            } else if attr.path.is_ident("persist") {
                match attr.parse_meta() {
                    Ok(syn::Meta::NameValue(syn::MetaNameValue {
                        lit: syn::Lit::Str(s),
                        ..
                    })) => {
                        if persist_attr.is_none() {
                            persist_attr = Some(s.value());
                        } else {
                            return syn::Error::new(attr.span(), "Duplicate persist attribute")
                                .to_compile_error()
                                .into();
                        }
                    }
                    _ => {
                        return syn::Error::new(
                            attr.span(),
                            "The persist attribute should be a key-value pair with a string argument: #[persist = \"foo_bar\"]",
                        )
                        .to_compile_error()
                        .into()
                    }
                };
            }
        }

        match (id_attr, persist_attr) {
            (Some(param_id), None) => {
                // The specific parameter types know how to convert themselves into the correct ParamPtr
                // variant
                param_mapping_insert_tokens
                    .push(quote! { param_map.insert(#param_id, self.#field_name.as_ptr()); });
            }
            (None, Some(stable_name)) => {
                // We don't know anything about the field types, but because we can generate this
                // function we get type erasure for free since we only need to worry about byte
                // vectors
                field_serialize_tokens.push(quote! {
                    match ::nih_plug::params::PersistentField::map(
                        &self.#field_name,
                        ::nih_plug::params::serialize_field,
                    ) {
                        Ok(data) => {
                            serialized.insert(String::from(#stable_name), data);
                        }
                        Err(err) => {
                            ::nih_plug::nih_log!("Could not serialize '{}': {}", #stable_name, err)
                        }
                    };
                });
                field_deserialize_tokens.push(quote! {
                    #stable_name => {
                        match ::nih_plug::params::deserialize_field(&data) {
                            Ok(deserialized) => {
                                ::nih_plug::params::PersistentField::set(
                                    &self.#field_name,
                                    deserialized,
                                );
                            }
                            Err(err) => {
                                ::nih_plug::nih_log!(
                                    "Could not deserialize '{}': {}",
                                    #stable_name,
                                    err
                                )
                            }
                        };
                    }
                });
            }
            (Some(_), Some(_)) => {
                return syn::Error::new(
                    field.span(),
                    "The id and persist attributes are mutually exclusive",
                )
                .to_compile_error()
                .into();
            }
            (None, None) => (),
        }
    }

    quote! {
        impl Params for #struct_name {
            fn param_map(
                self: std::pin::Pin<&Self>,
            ) -> std::collections::HashMap<&'static str, nih_plug::params::ParamPtr> {
                let mut param_map = std::collections::HashMap::new();

                #(#param_mapping_insert_tokens)*

                param_map
            }

            fn serialize_fields(&self) -> ::std::collections::HashMap<String, Vec<u8>> {
                let mut serialized = ::std::collections::HashMap::new();

                #(#field_serialize_tokens)*

                serialized
            }

            fn deserialize_fields(&self, serialized: &::std::collections::HashMap<String, Vec<u8>>) {
                for (field_name, data) in serialized {
                    match field_name.as_str() {
                        #(#field_deserialize_tokens)*
                        _ => nih_log!("Unknown field name: {}", field_name),
                    }
                }
            }
        }
    }
    .into()
}
