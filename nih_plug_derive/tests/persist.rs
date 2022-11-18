use std::collections::BTreeMap;
use std::sync::Mutex;

use nih_plug::prelude::*;

#[derive(Params, Default)]
struct WrapperParams {
    #[nested(id_prefix = "foo")]
    pub inner: InnerParams,
}

#[derive(Params, Default)]
struct ArrayWrapperParams {
    #[nested(array)]
    pub inners: [InnerParams; 3],
}

#[derive(Default)]
struct InnerParams {
    /// The value `deserialize()` has been called with so we can check that the prefix has been
    /// stripped correctly.
    pub deserialize_called_with: Mutex<Option<BTreeMap<String, String>>>,
}

unsafe impl Params for InnerParams {
    fn param_map(&self) -> Vec<(String, ParamPtr, String)> {
        Vec::new()
    }

    fn serialize_fields(&self) -> BTreeMap<String, String> {
        // When nested in another struct, the ID prefix will be added to `bar`
        let mut data = BTreeMap::new();
        data.insert(String::from("bar"), String::from("baz"));

        data
    }

    fn deserialize_fields(&self, serialized: &BTreeMap<String, String>) {
        *self.deserialize_called_with.lock().unwrap() = Some(serialized.clone());
    }
}

mod persist {
    mod nested_prefix {

        use super::super::*;

        #[test]
        fn serialize() {
            let params = WrapperParams::default();

            // This should have had a prefix added to the serialized value
            let serialized = params.serialize_fields();
            assert_eq!(serialized.len(), 1);
            assert_eq!(serialized["foo_bar"], "baz");
        }

        #[test]
        fn deserialize() {
            let mut serialized = BTreeMap::new();
            serialized.insert(String::from("foo_bar"), String::from("aaa"));

            let params = WrapperParams::default();
            params.deserialize_fields(&serialized);

            // This contains the values passed to the inner struct's deserialize function
            let deserialized = params
                .inner
                .deserialize_called_with
                .lock()
                .unwrap()
                .take()
                .unwrap();
            assert_eq!(deserialized.len(), 1);
            assert_eq!(deserialized["bar"], "aaa");
        }

        #[test]
        fn deserialize_mismatching_prefix() {
            let mut serialized = BTreeMap::new();
            serialized.insert(String::from("foo_bar"), String::from("aaa"));
            serialized.insert(
                String::from("something"),
                String::from("this should not be there"),
            );

            let params = WrapperParams::default();
            params.deserialize_fields(&serialized);

            // The `something` key should not be passed to the child struct
            let deserialized = params
                .inner
                .deserialize_called_with
                .lock()
                .unwrap()
                .take()
                .unwrap();
            assert_eq!(deserialized.len(), 1);
            assert_eq!(deserialized["bar"], "aaa");
        }
    }

    mod array_suffix {
        use super::super::*;

        #[test]
        fn serialize() {
            let params = ArrayWrapperParams::default();

            let serialized = params.serialize_fields();
            assert_eq!(serialized.len(), 3);
            assert_eq!(serialized["bar_1"], "baz");
            assert_eq!(serialized["bar_2"], "baz");
            assert_eq!(serialized["bar_2"], "baz");
        }

        #[test]
        fn deserialize() {
            let mut serialized = BTreeMap::new();
            serialized.insert(String::from("bar_1"), String::from("aaa"));
            serialized.insert(String::from("bar_2"), String::from("bbb"));
            serialized.insert(String::from("bar_3"), String::from("ccc"));

            let params = ArrayWrapperParams::default();
            params.deserialize_fields(&serialized);
            for (inner, expected_value) in params.inners.into_iter().zip(["aaa", "bbb", "ccc"]) {
                let deserialized = inner
                    .deserialize_called_with
                    .lock()
                    .unwrap()
                    .take()
                    .unwrap();
                assert_eq!(deserialized.len(), 1);
                assert_eq!(deserialized["bar"], expected_value);
            }
        }
    }
}
