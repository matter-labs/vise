//! Utils shared among multiple derive macros.

use syn::Attribute;

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
