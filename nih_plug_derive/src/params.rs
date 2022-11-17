use proc_macro::TokenStream;
use quote::quote;
use syn::spanned::Spanned;

pub fn derive_params(input: TokenStream) -> TokenStream {
    let ast = syn::parse_macro_input!(input as syn::DeriveInput);

    let struct_name = &ast.ident;
    let (impl_generics, ty_generics, where_clause) = ast.generics.split_for_impl();
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
    // parameters. For the `persist` function we'll create functions that serialize and deserialize
    // those fields individually (so they can be added and removed independently of eachother) using
    // JSON. The `nested` fields should also implement the `Params` trait and their fields will be
    // inherited and added to this field's lists.  We'll also enforce that there are no duplicate
    // keys at compile time.
    // TODO: This duplication check doesn't work for nested fields since we don't know anything
    //       about the fields on the nested structs
    let mut params: Vec<AnyParam> = Vec::new();
    let mut persistent_fields: Vec<PersistentField> = Vec::new();
    let mut group_params: Vec<NestedParams> = Vec::new();
    for field in fields.named {
        let field_name = match &field.ident {
            Some(ident) => ident,
            _ => continue,
        };

        // All attributes are mutually exclusive. If we encounter multiple or duplicate attributes,
        // then we'll error out.
        let mut processed_attribute = false;
        for attr in &field.attrs {
            if attr.path.is_ident("id") {
                match attr.parse_meta() {
                    Ok(syn::Meta::NameValue(syn::MetaNameValue {
                        lit: syn::Lit::Str(s),
                        ..
                    })) => {
                        if processed_attribute {
                            return syn::Error::new(
                                attr.span(),
                                "Duplicate or incompatible attribute found",
                            )
                            .to_compile_error()
                            .into();
                        }

                        // This is a vector since we want to preserve the order. If structs get
                        // large enough to the point where a linear search starts being expensive,
                        // then the plugin should probably start splitting up their parameters.
                        if params.iter().any(|p| match p {
                            AnyParam::Single(param) => param.id == s,
                            _ => false,
                        }) {
                            return syn::Error::new(
                                field.span(),
                                "Multiple parameters with the same ID found",
                            )
                            .to_compile_error()
                            .into();
                        }

                        params.push(AnyParam::Single(Param {
                            id: s,
                            field: field_name.clone(),
                        }));

                        processed_attribute = true;
                    }
                    _ => {
                        return syn::Error::new(
                            attr.span(),
                            "The id attribute should be a key-value pair with a string argument: \
                             #[id = \"foo_bar\"]",
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
                        if processed_attribute {
                            return syn::Error::new(
                                attr.span(),
                                "Duplicate or incompatible attribute found",
                            )
                            .to_compile_error()
                            .into();
                        }

                        if persistent_fields.iter().any(|p| p.key == s) {
                            return syn::Error::new(
                                field.span(),
                                "Multiple persistent fields with the same key found",
                            )
                            .to_compile_error()
                            .into();
                        }

                        persistent_fields.push(PersistentField {
                            key: s,
                            field: field_name.clone(),
                        });

                        processed_attribute = true;
                    }
                    _ => {
                        return syn::Error::new(
                            attr.span(),
                            "The persist attribute should be a key-value pair with a string \
                             argument: #[persist = \"foo_bar\"]",
                        )
                        .to_compile_error()
                        .into()
                    }
                };
            } else if attr.path.is_ident("nested") {
                // This one is more complicated. Support an `array` attribute, an `id_prefix =
                // "foo"` attribute, and a `group = "group name"` attribute. All are optional, and
                // the first two are mutually exclusive.
                let mut nested_array = false;
                let mut nested_id_prefix: Option<syn::LitStr> = None;
                let mut nested_group: Option<syn::LitStr> = None;
                match attr.parse_meta() {
                    Ok(syn::Meta::List(syn::MetaList {
                        nested: nested_attrs,
                        ..
                    })) => {
                        if processed_attribute {
                            return syn::Error::new(
                                attr.span(),
                                "Duplicate or incompatible attribute found",
                            )
                            .to_compile_error()
                            .into();
                        }

                        for nested_attr in nested_attrs {
                            match nested_attr {
                                syn::NestedMeta::Meta(syn::Meta::Path(p))
                                    if p.is_ident("array") =>
                                {
                                    nested_array = true;
                                }
                                syn::NestedMeta::Meta(syn::Meta::NameValue(
                                    syn::MetaNameValue {
                                        path,
                                        lit: syn::Lit::Str(s),
                                        ..
                                    },
                                )) if path.is_ident("id_prefix") => {
                                    nested_id_prefix = Some(s.clone());
                                }
                                syn::NestedMeta::Meta(syn::Meta::NameValue(
                                    syn::MetaNameValue {
                                        path,
                                        lit: syn::Lit::Str(s),
                                        ..
                                    },
                                )) if path.is_ident("group") => {
                                    let group_name = s.value();
                                    if group_name.is_empty() {
                                        return syn::Error::new(
                                            attr.span(),
                                            "Group names cannot be empty",
                                        )
                                        .to_compile_error()
                                        .into();
                                    } else if group_name.contains('/') {
                                        return syn::Error::new(
                                            attr.span(),
                                            "Group names may not contain slashes",
                                        )
                                        .to_compile_error()
                                        .into();
                                    } else {
                                        nested_group = Some(s.clone());
                                    }
                                }
                                _ => {
                                    return syn::Error::new(
                                        nested_attr.span(),
                                        "Unknown attribute. See the Params trait documentation \
                                         for more information.",
                                    )
                                    .to_compile_error()
                                    .into()
                                }
                            }
                        }

                        let param = match (nested_array, nested_id_prefix) {
                            (true, None) => NestedParams::Array {
                                field: field_name.clone(),
                                group: nested_group,
                            },
                            (false, Some(id_prefix)) => NestedParams::Prefixed {
                                field: field_name.clone(),
                                id_prefix,
                                group: nested_group,
                            },
                            (false, None) => NestedParams::Inline {
                                field: field_name.clone(),
                                group: nested_group,
                            },
                            (true, Some(_)) => {
                                return syn::Error::new(
                                    attr.span(),
                                    "'array' cannot be used together with 'id_prefix'",
                                )
                                .to_compile_error()
                                .into()
                            }
                        };

                        // Grouped parameters are handled differently than nested parameters:
                        // DAWs that support grouped parameters usually display parameter groups below "regular" ones
                        // in a tree view. To keep the order consistent with those, group parameters are collected
                        // separately, so they can be inserted at the end of the parameter list.
                        // Nested parameters without a group on the other hand are merely an implementation detail of
                        // the plugin and thus should not behave different than regular parameters to the user. Because
                        // of this, they are inserted alongside the "regular" ones.
                        if param.is_group() {
                            group_params.push(param);
                        } else {
                            params.push(AnyParam::Nested(param));
                        }

                        processed_attribute = true;
                    }
                    _ => {
                        return syn::Error::new(
                            attr.span(),
                            "The nested attribute should be a list in the following format: \
                             #[nested([array | id_prefix = \"foo\"], [group = \"group name\"])]",
                        )
                        .to_compile_error()
                        .into()
                    }
                };
            }
        }
    }

    // The next step is build the gathered information into tokens that can be spliced into a
    // `Params` implementation
    let param_map_tokens = {
        let param_mapping_self_tokens = params.into_iter().map(|p| p.to_token());
        let param_mapping_group_tokens = group_params.iter().map(|p| p.to_token());

        quote! {
            // This may not be in scope otherwise, used to call .as_ptr()
            use ::nih_plug::params::Param;

            #[allow(unused_mut)]
            let mut param_map = Vec::new();
            #(param_map.extend(#param_mapping_self_tokens); )*
            #(param_map.extend(#param_mapping_group_tokens); )*

            param_map
        }
    };

    let (serialize_fields_tokens, deserialize_fields_tokens) = {
        // Like with `param_map()`, we'll try to do the serialization for this struct and then
        // recursively call the child parameter structs. We don't know anything about the actual
        // field types, but because we can generate this function we can get type erasure for free
        // since we only need to worry about byte vectors.
        let (serialize_fields_self_tokens, deserialize_fields_match_self_tokens): (Vec<_>, Vec<_>) =
            persistent_fields
                .into_iter()
                .map(|PersistentField { field, key }| {
                    (
                        quote! {
                            match ::nih_plug::params::persist::PersistentField::map(
                                &self.#field,
                                ::nih_plug::params::persist::serialize_field,
                            ) {
                                Ok(data) => {
                                    serialized.insert(String::from(#key), data);
                                }
                                Err(err) => {
                                    ::nih_plug::nih_debug_assert_failure!(
                                        "Could not serialize '{}': {}",
                                        #key,
                                        err
                                    )
                                }
                            };
                        },
                        quote! {
                            #key => {
                                match ::nih_plug::params::persist::deserialize_field(&data) {
                                    Ok(deserialized) => {
                                        ::nih_plug::params::persist::PersistentField::set(
                                            &self.#field,
                                            deserialized,
                                        );
                                    }
                                    Err(err) => {
                                        ::nih_plug::nih_debug_assert_failure!(
                                            "Could not deserialize '{}': {}",
                                            #key,
                                            err
                                        )
                                    }
                                };
                            }
                        },
                    )
                })
                .unzip();

        let (serialize_fields_nested_tokens, deserialize_fields_nested_tokens): (Vec<_>, Vec<_>) =
            group_params
                .iter()
                .map(|nested| match nested {
                    NestedParams::Inline { field, .. } | NestedParams::Prefixed { field, .. } => (
                        // TODO: For some reason the macro won't parse correctly if you inline this
                        quote! {
                            let inlineme = self.#field.serialize_fields();
                            serialized.extend(inlineme);
                        },
                        quote! { self.#field.deserialize_fields(serialized) },
                    ),
                    NestedParams::Array { field, .. } => (
                        quote! {
                            for field in self.#field.iter() {
                                 serialized.extend(field.serialize_fields());
                            }
                        },
                        quote! {
                            for field in self.#field.iter() {
                                field.deserialize_fields(serialized);
                            }
                        },
                    ),
                })
                .unzip();

        let serialize_fields_tokens = quote! {
            #[allow(unused_mut)]
            let mut serialized = ::std::collections::BTreeMap::new();
            #(#serialize_fields_self_tokens);*

            #(#serialize_fields_nested_tokens);*

            serialized
        };

        let deserialize_fields_tokens = quote! {
            for (field_name, data) in serialized {
                match field_name.as_str() {
                    #(#deserialize_fields_match_self_tokens)*
                    _ => ::nih_plug::nih_debug_assert_failure!("Unknown serialized field name: {} (this may not be accurate)", field_name),
                }
            }

            // FIXME: The above warning will course give false postiives when using nested
            //        parameter structs. An easy fix would be to use
            //        https://doc.rust-lang.org/std/collections/struct.HashMap.html#method.drain_filter
            //        once that gets stabilized.
            #(#deserialize_fields_nested_tokens);*
        };

        (serialize_fields_tokens, deserialize_fields_tokens)
    };

    quote! {
        unsafe impl #impl_generics Params for #struct_name #ty_generics #where_clause {
            fn param_map(&self) -> Vec<(String, nih_plug::prelude::ParamPtr, String)> {
                #param_map_tokens
            }

            fn serialize_fields(&self) -> ::std::collections::BTreeMap<String, String> {
                #serialize_fields_tokens
            }

            fn deserialize_fields(&self, serialized: &::std::collections::BTreeMap<String, String>) {
                #deserialize_fields_tokens
            }
        }
    }
    .into()
}

#[derive(Debug)]
enum AnyParam {
    Single(Param),
    Nested(NestedParams),
}

impl AnyParam {
    fn to_token(&self) -> proc_macro2::TokenStream {
        match self {
            AnyParam::Single(Param { field, id }) => {
                quote! { [(String::from(#id), self.#field.as_ptr(), String::new())] }
            }
            AnyParam::Nested(params) => params.to_token(),
        }
    }
}

/// A parameter that should be added to the parameter map.
#[derive(Debug)]
struct Param {
    /// The name of the parameter's field on the struct.
    field: syn::Ident,
    /// The parameter's unique ID.
    id: syn::LitStr,
}

/// A field containing data that must be stored in the plugin's state.
#[derive(Debug)]
struct PersistentField {
    /// The name of the field on the struct.
    field: syn::Ident,
    /// The field's unique key.
    key: syn::LitStr,
}

/// A field containing another object whose parameters and persistent fields should be added to this
/// struct's.
#[derive(Debug)]
enum NestedParams {
    /// The nested struct's parameters are taken as is.
    Inline {
        field: syn::Ident,
        group: Option<syn::LitStr>,
    },
    /// The nested struct's parameters will get an ID prefix. The original parmaeter with ID `foo`
    /// will become `{id_prefix}_foo`.
    Prefixed {
        field: syn::Ident,
        id_prefix: syn::LitStr,
        group: Option<syn::LitStr>,
    },
    /// This field is an array-like data structure containing nested parameter structs. The
    /// parameter `foo` will get the new parameter ID `foo_{array_idx + 1}`, and if the group name
    /// is set then the group will be `{group_name} {array_idx + 1}`.
    Array {
        field: syn::Ident,
        group: Option<syn::LitStr>,
    },
}

impl NestedParams {
    fn is_group(&self) -> bool {
        let group = match self {
            NestedParams::Inline { field: _, group } => group,
            NestedParams::Prefixed {
                field: _,
                id_prefix: _,
                group,
            } => group,
            NestedParams::Array { field: _, group } => group,
        };

        group.is_some()
    }

    fn to_token(&self) -> proc_macro2::TokenStream {
        // How nested parameters are handled depends on the `NestedParams` variant.
        // These are pairs of `(parameter_id, param_ptr, param_group)`. The specific
        // parameter types know how to convert themselves into the correct ParamPtr variant.
        // Top-level parameters have no group, and we'll prefix the group name specified in
        // the `#[nested(...)]` attribute to fields coming from nested groups

        match self {
            // TODO: No idea how to splice this as an `Option<&str>`, so this involves some
            //       copy-pasting
            NestedParams::Inline {
                field,
                group: Some(group),
            } => quote! {
                self.#field.param_map().into_iter().map(|(param_id, param_ptr, nested_group_name)| {
                    if nested_group_name.is_empty() {
                        (param_id, param_ptr, String::from(#group))
                    } else {
                        (param_id, param_ptr, format!("{}/{}", #group, nested_group_name))
                    }
                })
            },
            NestedParams::Inline { field, group: None } => quote! {
                self.#field.param_map();
            },
            NestedParams::Prefixed {
                field,
                id_prefix,
                group: Some(group),
            } => quote! {
                self.#field.param_map().into_iter().map(|(param_id, param_ptr, nested_group_name)| {
                    let param_id = format!("{}_{}", #id_prefix, param_id);

                    if nested_group_name.is_empty() {
                        (param_id, param_ptr, String::from(#group))
                    } else {
                        (param_id, param_ptr, format!("{}/{}", #group, nested_group_name))
                    }
                })
            },
            NestedParams::Prefixed {
                field,
                id_prefix,
                group: None,
            } => quote! {
                self.#field.param_map().into_iter().map(|(param_id, param_ptr, nested_group_name)| {
                    let param_id = format!("{}_{}", #id_prefix, param_id);

                    (param_id, param_ptr, nested_group_name)
                })
            },
            // We'll start at index 1 for display purposes. Both the group and the parameter ID get
            // a suffix matching the array index.
            NestedParams::Array {
                field,
                group: Some(group),
            } => quote! {
                self.#field.iter().enumerate().flat_map(|(idx, params)| {
                    let idx = idx + 1;

                    params.param_map().into_iter().map(move |(param_id, param_ptr, nested_group_name)| {
                        let param_id = format!("{}_{}", param_id, idx);
                        let group = format!("{} {}", #group, idx);

                        // Note that this is different from the other variants
                        if nested_group_name.is_empty() {
                            (param_id, param_ptr, group)
                        } else {
                            (param_id, param_ptr, format!("{}/{}", group, nested_group_name))
                        }
                    })
                })
            },
            NestedParams::Array { field, group: None } => quote! {
                self.#field.iter().enumerate().flat_map(|(idx, params)| {
                    let idx = idx + 1;

                    params.param_map().into_iter().map(move |(param_id, param_ptr, nested_group_name)| {
                        let param_id = format!("{}_{}", param_id, idx);

                        (param_id, param_ptr, nested_group_name)
                    })
                })
            },
        }
    }
}
