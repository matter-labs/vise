//! Derivation of `EncodeLabelValue` and `EncodeLabelSet` traits.

use std::{collections::HashSet, fmt};

use proc_macro::TokenStream;
use quote::quote;
use syn::{Attribute, Data, DeriveInput, Field, Fields, Ident, LitStr, Path, PathArguments, Type};

use crate::utils::{metrics_attribute, validate_name, ParseAttribute};

#[derive(Debug, Clone, Copy)]
#[allow(clippy::enum_variant_names)]
enum RenameRule {
    LowerCase,
    UpperCase,
    CamelCase,
    SnakeCase,
    ScreamingSnakeCase,
    KebabCase,
    ScreamingKebabCase,
}

impl RenameRule {
    fn parse(s: &str) -> Result<Self, &'static str> {
        Ok(match s {
            "lowercase" => Self::LowerCase,
            "UPPERCASE" => Self::UpperCase,
            "camelCase" => Self::CamelCase,
            "snake_case" => Self::SnakeCase,
            "SCREAMING_SNAKE_CASE" => Self::ScreamingSnakeCase,
            "kebab-case" => Self::KebabCase,
            "SCREAMING-KEBAB-CASE" => Self::ScreamingKebabCase,
            _ => {
                return Err(
                    "Invalid case specified; should be one of: lowercase, UPPERCASE, camelCase, \
                     snake_case, SCREAMING_SNAKE_CASE, kebab-case, SCREAMING-KEBAB-CASE",
                )
            }
        })
    }

    fn transform(self, ident: &str) -> String {
        debug_assert!(ident.is_ascii()); // Should be checked previously
        let (spacing_char, scream) = match self {
            Self::LowerCase => return ident.to_ascii_lowercase(),
            Self::UpperCase => return ident.to_ascii_uppercase(),
            Self::CamelCase => return ident[..1].to_ascii_lowercase() + &ident[1..],
            // ^ Since `ident` is an ASCII string, indexing is safe
            Self::SnakeCase => ('_', false),
            Self::ScreamingSnakeCase => ('_', true),
            Self::KebabCase => ('-', false),
            Self::ScreamingKebabCase => ('-', true),
        };

        let mut output = String::with_capacity(ident.len());
        for (i, ch) in ident.char_indices() {
            if i > 0 && ch.is_ascii_uppercase() {
                output.push(spacing_char);
            }
            output.push(if scream {
                ch.to_ascii_uppercase()
            } else {
                ch.to_ascii_lowercase()
            });
        }
        output
    }
}

#[derive(Default)]
struct EncodeLabelAttrs {
    cr: Option<Path>,
    rename_all: Option<RenameRule>,
    format: Option<LitStr>,
    label: Option<LitStr>,
}

impl fmt::Debug for EncodeLabelAttrs {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EncodeLabelAttrs")
            .field("cr", &self.cr.as_ref().map(|_| "_"))
            .field("rename_all", &self.rename_all)
            .field("format", &self.format.as_ref().map(LitStr::value))
            .field("label", &self.label.as_ref().map(LitStr::value))
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
            } else if meta.path.is_ident("rename_all") {
                let case_str: LitStr = meta.value()?.parse()?;
                let case = RenameRule::parse(&case_str.value())
                    .map_err(|message| syn::Error::new(case_str.span(), message))?;
                attrs.rename_all = Some(case);
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
struct EnumVariant {
    ident: Ident,
    label_value: String,
}

impl EnumVariant {
    fn encode(&self) -> proc_macro2::TokenStream {
        let ident = &self.ident;
        let label_value = &self.label_value;
        quote!(Self::#ident => #label_value)
    }
}

#[derive(Default)]
struct EnumVariantAttrs {
    name: Option<LitStr>,
}

impl fmt::Debug for EnumVariantAttrs {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EnumVariantAttrs")
            .field("name", &self.name.as_ref().map(LitStr::value))
            .finish()
    }
}

impl ParseAttribute for EnumVariantAttrs {
    fn parse(raw: &Attribute) -> syn::Result<Self> {
        let mut attrs = Self::default();
        raw.parse_nested_meta(|meta| {
            if meta.path.is_ident("name") {
                attrs.name = Some(meta.value()?.parse()?);
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
    enum_variants: Option<Vec<EnumVariant>>,
}

impl EncodeLabelValueImpl {
    fn new(raw: &DeriveInput) -> syn::Result<Self> {
        let attrs: EncodeLabelAttrs = metrics_attribute(&raw.attrs)?;
        if let Some(format) = &attrs.format {
            if attrs.rename_all.is_some() {
                let message = "`rename_all` and `format` attributes cannot be specified together";
                return Err(syn::Error::new(format.span(), message));
            }
        }

        let enum_variants = attrs
            .rename_all
            .map(|case| Self::extract_enum_variants(raw, case))
            .transpose()?;

        Ok(Self {
            attrs,
            enum_variants,
            name: raw.ident.clone(),
        })
    }

    fn extract_enum_variants(raw: &DeriveInput, case: RenameRule) -> syn::Result<Vec<EnumVariant>> {
        let Data::Enum(data) = &raw.data else {
            let message = "`rename_all` attribute can only be placed on enums";
            return Err(syn::Error::new_spanned(raw, message));
        };

        let mut unique_label_values = HashSet::with_capacity(data.variants.len());
        let variants = data.variants.iter().map(|variant| {
            if !matches!(variant.fields, Fields::Unit) {
                let message = "To use `rename_all` attribute, all enum variants must be plain \
                    (have no fields)";
                return Err(syn::Error::new_spanned(variant, message));
            }
            let ident_str = variant.ident.to_string();
            if !ident_str.is_ascii() {
                let message = "Variant name must consist of ASCII chars";
                return Err(syn::Error::new(variant.ident.span(), message));
            }
            let attrs: EnumVariantAttrs = metrics_attribute(&variant.attrs)?;
            let label_value = if let Some(name_override) = attrs.name {
                name_override.value()
            } else {
                case.transform(&ident_str)
            };
            if !unique_label_values.insert(label_value.clone()) {
                let message = format!("Label value `{label_value}` is redefined");
                return Err(syn::Error::new_spanned(variant, message));
            }

            Ok(EnumVariant {
                ident: variant.ident.clone(),
                label_value,
            })
        });
        variants.collect()
    }

    fn impl_value(&self) -> proc_macro2::TokenStream {
        let cr = if let Some(cr) = &self.attrs.cr {
            quote!(#cr)
        } else {
            quote!(vise)
        };
        let name = &self.name;
        let encoding = quote!(#cr::_reexports::encoding);

        let encode_impl = if let Some(enum_variants) = &self.enum_variants {
            let variant_hands = enum_variants.iter().map(EnumVariant::encode);
            quote! {
                use core::fmt::Write as _;
                core::write!(encoder, "{}", match self {
                    #(#variant_hands,)*
                })
            }
        } else {
            let format_lit;
            let format = if let Some(format) = &self.attrs.format {
                format
            } else {
                format_lit = LitStr::new("{}", name.span());
                &format_lit
            };

            quote! {
                use core::fmt::Write as _;
                core::write!(encoder, #format, self)
            }
        };

        quote! {
            impl #encoding::EncodeLabelValue for #name {
                fn encode(
                    &self,
                    encoder: &mut #encoding::LabelValueEncoder<'_>,
                ) -> core::fmt::Result {
                    #encode_impl
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

        let this = Self {
            name,
            is_option: Self::detect_is_option(&raw.ty),
            attrs: metrics_attribute(&raw.attrs)?,
        };
        validate_name(&this.label_string())
            .map_err(|message| syn::Error::new(this.name.span(), message))?;
        Ok(this)
    }

    /// Strips the `r#` prefix from raw identifiers.
    fn label_string(&self) -> String {
        let label = self.name.to_string();
        if let Some(stripped) = label.strip_prefix("r#") {
            stripped.to_owned()
        } else {
            label
        }
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
        let label = LitStr::new(&self.label_string(), name.span());

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
        let EncodeLabelValueImpl { attrs, name, .. } = EncodeLabelValueImpl::new(raw)?;

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renaming_rules() {
        let ident = "TestIdent";
        let rules_and_expected_outcomes = [
            (RenameRule::LowerCase, "testident"),
            (RenameRule::UpperCase, "TESTIDENT"),
            (RenameRule::CamelCase, "testIdent"),
            (RenameRule::SnakeCase, "test_ident"),
            (RenameRule::ScreamingSnakeCase, "TEST_IDENT"),
            (RenameRule::KebabCase, "test-ident"),
            (RenameRule::ScreamingKebabCase, "TEST-IDENT"),
        ];
        for (rule, expected) in rules_and_expected_outcomes {
            assert_eq!(rule.transform(ident), expected);
        }
    }

    #[test]
    fn encoding_label_set() {
        let input: DeriveInput = syn::parse_quote! {
            struct TestLabels {
                r#type: &'static str,
                #[metrics(skip = str::is_empty)]
                kind: &'static str,
            }
        };
        let label_set = EncodeLabelSetImpl::new(&input).unwrap();
        let fields = label_set.fields.as_ref().unwrap();
        assert_eq!(fields.len(), 2);
        assert_eq!(fields[0].label_string(), "type");
        assert_eq!(fields[1].label_string(), "kind");
        assert!(fields[1].attrs.skip.is_some());
    }

    #[test]
    fn label_value_redefinition_error() {
        let input: DeriveInput = syn::parse_quote! {
            #[metrics(rename_all = "snake_case")]
            enum Label {
                First,
                #[metrics(name = "first")]
                Second,
            }
        };
        let err = EncodeLabelValueImpl::new(&input).unwrap_err().to_string();
        assert!(err.contains("Label value `first` is redefined"), "{err}");
    }
}
