//! `register` attribute macro.

use proc_macro::TokenStream;
use quote::quote;
use syn::{Attribute, ItemStatic, Path};

use std::fmt;

use crate::utils::{metrics_attribute, ParseAttribute};

/// `#[metrics(..)]` attributes on registered static items.
#[derive(Default)]
struct RegistrationAttrs {
    cr: Option<Path>,
}

impl fmt::Debug for RegistrationAttrs {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RegistrationAttrs")
            .field("cr", &self.cr.as_ref().map(|_| "_"))
            .finish()
    }
}

impl ParseAttribute for RegistrationAttrs {
    fn parse(raw: &Attribute) -> syn::Result<Self> {
        let mut attrs = Self::default();
        raw.parse_nested_meta(|meta| {
            if meta.path.is_ident("crate") {
                attrs.cr = Some(meta.value()?.parse()?);
                Ok(())
            } else {
                Err(meta.error("unsupported attribute"))
            }
        })?;
        Ok(attrs)
    }
}

fn register_static(input: &mut ItemStatic) -> syn::Result<proc_macro2::TokenStream> {
    let attrs: RegistrationAttrs = metrics_attribute(&input.attrs)?;
    input
        .attrs
        .retain(|attr| !attr.meta.path().is_ident("metrics"));

    let cr = if let Some(cr) = &attrs.cr {
        quote!(#cr)
    } else {
        quote!(vise)
    };
    let name = &input.ident;

    Ok(quote! {
        const _: () = {
            #[#cr::_reexports::linkme::distributed_slice(#cr::METRICS_REGISTRATIONS)]
            #[linkme(crate = #cr::_reexports::linkme)]
            fn __register(registry: &mut #cr::Registry) {
                #cr::CollectToRegistry::collect_to_registry(&#name, registry);
            }
        };
    })
}

pub(crate) fn impl_register(_attrs: TokenStream, input: TokenStream) -> TokenStream {
    let mut item: ItemStatic = match syn::parse(input) {
        Ok(item) => item,
        Err(err) => return err.into_compile_error().into(),
    };

    match register_static(&mut item) {
        Ok(registration) => quote! {
            #item
            #registration
        }
        .into(),
        Err(err) => err.into_compile_error().into(),
    }
}
