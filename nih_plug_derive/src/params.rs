use proc_macro::TokenStream;
use quote::quote;
use std::collections::HashSet;
use syn::spanned::Spanned;

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

    // We only care about fields with `id`, `persist`, and `nested` attributes. For the `id` fields
    // we'll build a mapping function that creates a hashmap containing pointers to those
    // parmaeters. For the `persist` function we'll create functions that serialize and deserialize
    // those fields individually (so they can be added and removed independently of eachother) using
    // JSON. The `nested` fields should also implement the `Params` trait and their fields will be
    // inherited and added to this field's lists.
    let mut param_mapping_insert_tokens = Vec::new();
    let mut param_id_string_tokens = Vec::new();
    let mut field_serialize_tokens = Vec::new();
    let mut field_deserialize_tokens = Vec::new();
    let mut nested_fields_idents = Vec::new();

    // We'll also enforce that there are no duplicate keys at compile time
    // TODO: This doesn't work for nested fields since we don't know anything about the fields on
    //       the nested structs
    let mut param_ids = HashSet::new();
    let mut persist_ids = HashSet::new();
    for field in fields.named {
        let field_name = match &field.ident {
            Some(ident) => ident,
            _ => continue,
        };

        // These two attributes are mutually exclusive
        let mut id_attr: Option<String> = None;
        let mut persist_attr: Option<String> = None;
        let mut nested = false;
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
            } else if attr.path.is_ident("nested") {
                match attr.parse_meta() {
                    Ok(syn::Meta::Path(_)) => {
                        if !nested {
                            nested = true;
                        } else {
                            return syn::Error::new(attr.span(), "Duplicate nested attribute")
                                .to_compile_error()
                                .into();
                        }
                    }
                    _ => {
                        return syn::Error::new(
                            attr.span(),
                            "The nested attribute should not have any arguments: #[nested]",
                        )
                        .to_compile_error()
                        .into();
                    }
                };
            }
        }

        match (id_attr, persist_attr) {
            (Some(param_id), None) => {
                if !param_ids.insert(param_id.clone()) {
                    return syn::Error::new(
                        field.span(),
                        "Multiple fields with the same parameter ID found",
                    )
                    .to_compile_error()
                    .into();
                }

                // The specific parameter types know how to convert themselves into the correct ParamPtr
                // variant
                param_mapping_insert_tokens
                    .push(quote! { param_map.insert(#param_id, self.#field_name.as_ptr()); });
                param_id_string_tokens.push(quote! { #param_id, });
            }
            (None, Some(stable_name)) => {
                if !persist_ids.insert(stable_name.clone()) {
                    return syn::Error::new(
                        field.span(),
                        "Multiple persisted fields with the same ID found",
                    )
                    .to_compile_error()
                    .into();
                }

                // We don't know anything about the field types, but because we can generate this
                // function we get type erasure for free since we only need to worry about byte
                // vectors
                field_serialize_tokens.push(quote! {
                    match ::nih_plug::param::internals::PersistentField::map(
                        &self.#field_name,
                        ::nih_plug::param::internals::serialize_field,
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
                        match ::nih_plug::param::internals::deserialize_field(&data) {
                            Ok(deserialized) => {
                                ::nih_plug::param::internals::PersistentField::set(
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

        if nested {
            nested_fields_idents.push(field_name.clone());
        }
    }

    quote! {
        impl Params for #struct_name {
            fn param_map(
                self: std::pin::Pin<&Self>,
            ) -> std::collections::HashMap<&'static str, nih_plug::param::internals::ParamPtr> {
                // This may not be in scope otherwise
                use ::nih_plug::param::Param;

                let mut param_map = std::collections::HashMap::new();
                #(#param_mapping_insert_tokens)*

                let nested_fields: &[&dyn Params] = &[#(&self.#nested_fields_idents),*];
                for nested_params in nested_fields {
                    unsafe { param_map.extend(Pin::new_unchecked(*nested_params).param_map()) };
                }

                param_map
            }

            fn param_ids(self: std::pin::Pin<&Self>) -> Vec<&'static str> {
                let mut ids = vec![#(#param_id_string_tokens)*];

                let nested_fields: &[&dyn Params] = &[#(&self.#nested_fields_idents),*];
                for nested_params in nested_fields {
                    unsafe { ids.append(&mut Pin::new_unchecked(*nested_params).param_ids()) };
                }

                ids
            }

            fn serialize_fields(&self) -> ::std::collections::HashMap<String, String> {
                let mut serialized = ::std::collections::HashMap::new();
                #(#field_serialize_tokens)*

                let nested_fields: &[&dyn Params] = &[#(&self.#nested_fields_idents),*];
                for nested_params in nested_fields {
                    unsafe { serialized.extend(Pin::new_unchecked(*nested_params).serialize_fields()) };
                }

                serialized
            }

            fn deserialize_fields(&self, serialized: &::std::collections::HashMap<String, String>) {
                for (field_name, data) in serialized {
                    match field_name.as_str() {
                        #(#field_deserialize_tokens)*
                        _ => ::nih_plug::nih_log!("Unknown serialized field name: {} (this may not be accurate)", field_name),
                    }
                }

                // FIXME: The above warning will course give false postiives when using nested
                //        parameter structs. An easy fix would be to use
                //        https://doc.rust-lang.org/std/collections/struct.HashMap.html#method.drain_filter
                //        once that gets stabilized.
                let nested_fields: &[&dyn Params] = &[#(&self.#nested_fields_idents),*];
                for nested_params in nested_fields {
                    unsafe { Pin::new_unchecked(*nested_params).deserialize_fields(serialized) };
                }
            }
        }
    }
    .into()
}
