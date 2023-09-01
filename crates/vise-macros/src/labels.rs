//! Derivation of `EncodeLabelValue` and `EncodeLabelSet` traits.

use std::fmt;

use proc_macro::TokenStream;
use quote::quote;
use syn::{Attribute, Data, DeriveInput, Field, Ident, LitStr, Path, PathArguments, Type};

use crate::utils::{metrics_attribute, validate_name, ParseAttribute};

#[derive(Default)]
struct EncodeLabelAttrs {
    cr: Option<Path>,
    format: Option<LitStr>,
    label: Option<LitStr>,
}

impl fmt::Debug for EncodeLabelAttrs {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EncodeLabelAttrs")
            .field("cr", &self.cr.as_ref().map(|_| "_"))
            .field("format", &self.format.as_ref().map(|_| "_"))
            .field("label", &self.label.as_ref().map(|_| "_"))
            .finish()
    }
}

impl ParseAttribute for EncodeLabelAttrs {
    fn parse(raw: &Attribute) -> syn::Result<Self> {
        let mut attrs = Self::default();
        raw.parse_nested_meta(|meta| {
            if meta.path.is_ident("crate") {
                attrs.cr = Some(meta.value()?.parse()?);
                Ok(())
            } else if meta.path.is_ident("format") {
                attrs.format = Some(meta.value()?.parse()?);
                Ok(())
            } else if meta.path.is_ident("label") {
                let label: LitStr = meta.value()?.parse()?;
                validate_name(&label.value()).map_err(|message| meta.error(message))?;
                attrs.label = Some(label);
                Ok(())
            } else {
                Err(meta.error("unsupported attribute"))
            }
        })?;
        Ok(attrs)
    }
}

#[derive(Debug)]
struct EncodeLabelValueImpl {
    attrs: EncodeLabelAttrs,
    name: Ident,
}

impl EncodeLabelValueImpl {
    fn new(raw: &DeriveInput) -> syn::Result<Self> {
        Ok(Self {
            attrs: metrics_attribute(&raw.attrs)?,
            name: raw.ident.clone(),
        })
    }

    fn impl_value(&self) -> proc_macro2::TokenStream {
        let cr = if let Some(cr) = &self.attrs.cr {
            quote!(#cr)
        } else {
            quote!(vise)
        };
        let name = &self.name;
        let encoding = quote!(#cr::_reexports::encoding);

        let format_lit;
        let format = if let Some(format) = &self.attrs.format {
            format
        } else {
            format_lit = LitStr::new("{}", name.span());
            &format_lit
        };

        quote! {
            impl #encoding::EncodeLabelValue for #name {
                fn encode(
                    &self,
                    encoder: &mut #encoding::LabelValueEncoder<'_>,
                ) -> core::fmt::Result {
                    use core::fmt::Write as _;
                    core::write!(encoder, #format, self)
                }
            }
        }
    }
}

#[derive(Default)]
struct LabelFieldAttrs {
    skip: Option<Path>,
}

impl fmt::Debug for LabelFieldAttrs {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LabelFieldAttrs")
            .field("skip", &self.skip.as_ref().map(|_| ".."))
            .finish()
    }
}

impl ParseAttribute for LabelFieldAttrs {
    fn parse(raw: &Attribute) -> syn::Result<Self> {
        let mut attrs = Self::default();
        raw.parse_nested_meta(|meta| {
            if meta.path.is_ident("skip") {
                attrs.skip = Some(meta.value()?.parse()?);
                Ok(())
            } else {
                Err(meta.error("unsupported attribute"))
            }
        })?;
        Ok(attrs)
    }
}

#[derive(Debug)]
struct LabelField {
    name: Ident,
    is_option: bool,
    attrs: LabelFieldAttrs,
}

impl LabelField {
    fn parse(raw: &Field) -> syn::Result<Self> {
        let name = raw.ident.clone().ok_or_else(|| {
            let message = "Encoded fields must be named";
            syn::Error::new_spanned(raw, message)
        })?;
        validate_name(&name.to_string())
            .map_err(|message| syn::Error::new(name.span(), message))?;

        Ok(Self {
            name,
            is_option: Self::detect_is_option(&raw.ty),
            attrs: metrics_attribute(&raw.attrs)?,
        })
    }

    fn detect_is_option(ty: &Type) -> bool {
        let Type::Path(ty) = ty else {
            return false;
        };
        if ty.path.segments.len() != 1 {
            return false;
        }
        let first_segment = ty.path.segments.first().unwrap();
        first_segment.ident == "Option"
            && matches!(
                &first_segment.arguments,
                PathArguments::AngleBracketed(args) if args.args.len() == 1
            )
    }

    fn encode(&self, encoding: &proc_macro2::TokenStream) -> proc_macro2::TokenStream {
        let name = &self.name;
        let label = LitStr::new(&self.name.to_string(), name.span());

        // Skip `Option`al fields by default if they are `None`.
        let default_skip: Path;
        let skip = if self.is_option && self.attrs.skip.is_none() {
            default_skip = syn::parse_quote_spanned! {name.span()=>
                core::option::Option::is_none
            };
            Some(&default_skip)
        } else {
            self.attrs.skip.as_ref()
        };

        let encode_inner = quote! {
            let mut label_encoder = encoder.encode_label();
            let mut key_encoder = label_encoder.encode_label_key()?;
            #encoding::EncodeLabelKey::encode(&#label, &mut key_encoder)?;
            let mut value_encoder = key_encoder.encode_label_value()?;
            #encoding::EncodeLabelValue::encode(&self.#name, &mut value_encoder)?;
            value_encoder.finish()?;
        };
        if let Some(skip) = skip {
            quote! {
                if !#skip(&self.#name) {
                    #encode_inner
                }
            }
        } else {
            encode_inner
        }
    }
}

#[derive(Debug)]
struct EncodeLabelSetImpl {
    attrs: EncodeLabelAttrs,
    name: Ident,
    fields: Option<Vec<LabelField>>,
}

impl EncodeLabelSetImpl {
    fn new(raw: &DeriveInput) -> syn::Result<Self> {
        let EncodeLabelValueImpl { attrs, name } = EncodeLabelValueImpl::new(raw)?;

        let fields = if attrs.label.is_some() {
            None
        } else {
            let Data::Struct(data) = &raw.data else {
                let message = "Non-singleton `EncodeLabelSet` can only be derived on structs";
                return Err(syn::Error::new_spanned(raw, message));
            };
            let fields: syn::Result<_> = data.fields.iter().map(LabelField::parse).collect();
            Some(fields?)
        };

        Ok(Self {
            attrs,
            name,
            fields,
        })
    }

    fn impl_set(&self) -> proc_macro2::TokenStream {
        let cr = if let Some(cr) = &self.attrs.cr {
            quote!(#cr)
        } else {
            quote!(vise)
        };
        let name = &self.name;
        let encoding = quote!(#cr::_reexports::encoding);

        if let Some(label) = &self.attrs.label {
            quote! {
                impl #encoding::EncodeLabelSet for #name {
                    fn encode(
                        &self,
                        mut encoder: #encoding::LabelSetEncoder<'_>,
                    ) -> core::fmt::Result {
                        let mut label_encoder = encoder.encode_label();
                        let mut key_encoder = label_encoder.encode_label_key()?;
                        #encoding::EncodeLabelKey::encode(&#label, &mut key_encoder)?;
                        let mut value_encoder = key_encoder.encode_label_value()?;
                        #encoding::EncodeLabelValue::encode(self, &mut value_encoder)?;
                        value_encoder.finish()
                    }
                }
            }
        } else {
            let fields = self.fields.as_ref().unwrap();
            let fields = fields.iter().map(|field| field.encode(&encoding));

            quote! {
                impl #encoding::EncodeLabelSet for #name {
                    fn encode(
                        &self,
                        mut encoder: #encoding::LabelSetEncoder<'_>,
                    ) -> core::fmt::Result {
                        #(#fields)*
                        core::fmt::Result::Ok(())
                    }
                }
            }
        }
    }
}

pub(crate) fn impl_encode_label_value(input: TokenStream) -> TokenStream {
    let input: DeriveInput = syn::parse(input).unwrap();
    let trait_impl = match EncodeLabelValueImpl::new(&input) {
        Ok(trait_impl) => trait_impl,
        Err(err) => return err.into_compile_error().into(),
    };
    trait_impl.impl_value().into()
}

pub(crate) fn impl_encode_label_set(input: TokenStream) -> TokenStream {
    let input: DeriveInput = syn::parse(input).unwrap();
    let trait_impl = match EncodeLabelSetImpl::new(&input) {
        Ok(trait_impl) => trait_impl,
        Err(err) => return err.into_compile_error().into(),
    };
    trait_impl.impl_set().into()
}
