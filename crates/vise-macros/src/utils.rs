//! Utils shared among multiple derive macros.

use syn::{Attribute, Generics};

pub(crate) trait ParseAttribute: Sized {
    fn parse(raw: &Attribute) -> syn::Result<Self>;
}

pub(crate) fn metrics_attribute<T>(raw_attrs: &[Attribute]) -> syn::Result<T>
where
    T: ParseAttribute + Default,
{
    let attrs = raw_attrs
        .iter()
        .find(|attr| attr.meta.path().is_ident("metrics"));
    attrs.map_or_else(|| Ok(T::default()), T::parse)
}

pub(crate) fn ensure_no_generics(generics: &Generics, derived_macro: &str) -> syn::Result<()> {
    if generics.params.is_empty() {
        Ok(())
    } else {
        let message = format!("Generics are not supported for `derive({derived_macro})` macro");
        Err(syn::Error::new_spanned(generics, message))
    }
}
