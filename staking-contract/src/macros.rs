macro_rules! impl_versioned_value {
    ($versioned:ident, $value:ty, $current_variant:ident, [$($variant:ident),+ $(,)?]) => {
        impl From<$value> for $versioned {
            fn from(value: $value) -> Self {
                Self::$current_variant(value)
            }
        }

        impl From<$versioned> for $value {
            fn from(value: $versioned) -> Self {
                match value {
                    $($versioned::$variant(inner) => inner,)+
                }
            }
        }
    };
}

macro_rules! impl_versioned_lookup_accessors {
    ($get_name:ident, $set_name:ident, $field:ident, $key:ty, $value:ty) => {
        pub(crate) fn $get_name(&self, id: &$key) -> Option<$value> {
            self.$field.get(id).cloned().map(Into::into)
        }

        pub(crate) fn $set_name(&mut self, id: $key, value: $value) {
            self.$field.insert(id, value.into());
        }
    };
}

macro_rules! impl_versioned_lookup_getter {
    ($get_name:ident, $field:ident, $key:ty, $value:ty) => {
        pub(crate) fn $get_name(&self, id: &$key) -> Option<$value> {
            self.$field.get(id).cloned().map(Into::into)
        }
    };
}
