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

fn is_valid_name_char(ch: char) -> bool {
    ch == '_' || ch.is_ascii_lowercase() || ch.is_ascii_digit()
}

pub(crate) fn validate_name(name: &str) -> Result<(), &'static str> {
    if name.is_empty() {
        Err("name cannot be empty")
    } else if !name.starts_with(|ch: char| ch.is_ascii_lowercase()) {
        Err("name must start with a lower-case ASCII char (a-z)")
    } else if !name.chars().skip(1).all(is_valid_name_char) {
        Err("name must contain only lower-case ASCII chars, digits and underscores (a-z0-9_)")
    } else {
        Ok(())
    }
}
