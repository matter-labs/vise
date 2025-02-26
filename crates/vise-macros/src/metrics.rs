//! Derivation of the `Metrics` trait.

use std::fmt;

use proc_macro::TokenStream;
use quote::{quote, quote_spanned};
use syn::{
    spanned::Spanned, Attribute, Data, DeriveInput, Expr, Field, Ident, Lit, LitStr, Path, Type,
};

use crate::utils::{ensure_no_generics, metrics_attribute, ParseAttribute};

/// Struct-level `#[metrics(..)]` attributes.
#[derive(Default)]
struct MetricsAttrs {
    cr: Option<Path>,
    prefix: Option<LitStr>,
}

impl MetricsAttrs {
    fn path_to_crate(&self, span: proc_macro2::Span) -> proc_macro2::TokenStream {
        if let Some(cr) = &self.cr {
            // Overriding the span for `cr` via `quote_spanned!` doesn't work.
            quote!(#cr)
        } else {
            quote_spanned!(span=> vise)
        }
    }
}

impl fmt::Debug for MetricsAttrs {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MetricsAttrs")
            .finish_non_exhaustive()
    }
}

impl ParseAttribute for MetricsAttrs {
    fn parse(raw: &Attribute) -> syn::Result<Self> {
        let mut attrs = Self::default();
        raw.parse_nested_meta(|meta| {
            if meta.path.is_ident("crate") {
                attrs.cr = Some(meta.value()?.parse()?);
                Ok(())
            } else if meta.path.is_ident("prefix") {
                attrs.prefix = Some(meta.value()?.parse()?);
                Ok(())
            } else {
                Err(meta.error(
                    "Unsupported attribute; only `prefix` and `crate` attributes are supported \
                     (see `vise` crate docs for details)",
                ))
            }
        })?;
        Ok(attrs)
    }
}

#[derive(Default)]
struct MetricsFieldAttrs {
    buckets: Option<Expr>,
    unit: Option<Expr>,
    labels: Option<Expr>,
}

impl fmt::Debug for MetricsFieldAttrs {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MetricsFieldAttrs")
            .field("buckets", &self.buckets.as_ref().map(|_| ".."))
            .field("unit", &self.unit.as_ref().map(|_| ".."))
            .field("labels", &self.labels.as_ref().map(|_| ".."))
            .finish()
    }
}

impl ParseAttribute for MetricsFieldAttrs {
    fn parse(raw: &Attribute) -> syn::Result<Self> {
        let mut attrs = Self::default();
        raw.parse_nested_meta(|meta| {
            if meta.path.is_ident("buckets") {
                attrs.buckets = Some(meta.value()?.parse()?);
                Ok(())
            } else if meta.path.is_ident("unit") {
                attrs.unit = Some(meta.value()?.parse()?);
                Ok(())
            } else if meta.path.is_ident("labels") {
                attrs.labels = Some(meta.value()?.parse()?);
                Ok(())
            } else {
                Err(meta.error(
                    "Unsupported attribute; only `buckets`, `unit` and `labels` attributes are supported \
                     (see `vise` crate docs for details)"
                ))
            }
        })?;
        Ok(attrs)
    }
}

struct MetricsField {
    attrs: MetricsFieldAttrs,
    name: Ident,
    ty: Type,
    docs: String,
}

impl fmt::Debug for MetricsField {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MetricsField")
            .field("attrs", &self.attrs)
            .field("name", &self.name)
            .field("docs", &self.docs)
            .finish_non_exhaustive()
    }
}

impl MetricsField {
    fn parse(raw: &Field) -> syn::Result<Self> {
        let name = raw.ident.clone().ok_or_else(|| {
            let message = "Only named fields are supported";
            syn::Error::new_spanned(raw, message)
        })?;
        let ty = raw.ty.clone();
        let attrs = metrics_attribute(&raw.attrs)?;

        let doc_lines = raw.attrs.iter().filter_map(|attr| {
            if attr.meta.path().is_ident("doc") {
                let name_value = attr.meta.require_name_value().ok()?;
                let Expr::Lit(doc_literal) = &name_value.value else {
                    return None;
                };
                match &doc_literal.lit {
                    Lit::Str(doc_literal) => Some(doc_literal.value()),
                    _ => None,
                }
            } else {
                None
            }
        });

        let mut docs = String::new();
        for line in doc_lines {
            let line = line.trim();
            if !line.is_empty() {
                if !docs.is_empty() {
                    docs.push(' ');
                }
                docs.push_str(line);
            }
        }
        if docs.ends_with(['.', '!', '?']) {
            // Remove the trailing punctuation since it'll be inserted automatically by the `Registry`.
            docs.pop();
        }

        Ok(Self {
            attrs,
            name,
            ty,
            docs,
        })
    }

    fn initialize_default(&self, cr: &proc_macro2::TokenStream) -> proc_macro2::TokenStream {
        let name = &self.name;
        let span = self.ty.span();
        let mut builder = quote_spanned!(span=> #cr::MetricBuilder::new());
        if let Some(buckets) = &self.attrs.buckets {
            builder = quote_spanned!(span=> #builder.with_buckets(#buckets));
        }
        if let Some(labels) = &self.attrs.labels {
            builder = quote_spanned!(span=> #builder.with_labels(#labels));
        }

        quote_spanned! {span=>
            #name: #cr::BuildMetric::build(#builder)
        }
    }

    fn visit(&self, prefix: Option<&str>) -> proc_macro2::TokenStream {
        let name = &self.name;
        let name_str = if let Some(prefix) = prefix {
            format!("{prefix}_{name}")
        } else {
            name.to_string()
        };
        let docs = &self.docs;

        let unit = if let Some(unit) = &self.attrs.unit {
            quote!(core::option::Option::Some(#unit))
        } else {
            quote!(core::option::Option::None)
        };

        quote! {
            visitor.visit_metric(
                #name_str,
                #docs,
                #unit,
                ::std::boxed::Box::new(::core::clone::Clone::clone(&self.#name)),
            );
        }
    }

    fn describe(
        &self,
        prefix: Option<&str>,
        cr: &proc_macro2::TokenStream,
    ) -> proc_macro2::TokenStream {
        let name = &self.name;
        let name_str = if let Some(prefix) = prefix {
            format!("{prefix}_{name}")
        } else {
            name.to_string()
        };
        let docs = &self.docs;
        let ty = &self.ty;
        let unit = if let Some(unit) = &self.attrs.unit {
            quote!(core::option::Option::Some(#unit))
        } else {
            quote!(core::option::Option::None)
        };

        quote! {
            #cr::descriptors::MetricDescriptor {
                name: #name_str,
                field_name: core::stringify!(#name),
                metric_type: <#ty as #cr::_reexports::TypedMetric>::TYPE,
                help: #docs,
                unit: #unit,
            }
        }
    }
}

#[derive(Debug)]
struct MetricsImpl {
    attrs: MetricsAttrs,
    name: Ident,
    fields: Vec<MetricsField>,
}

impl MetricsImpl {
    fn new(input: &DeriveInput) -> syn::Result<Self> {
        ensure_no_generics(&input.generics, "Metrics")?;
        let Data::Struct(data) = &input.data else {
            let message = "#[derive(Metrics)] can only be placed on structs";
            return Err(syn::Error::new_spanned(input, message));
        };

        let attrs = metrics_attribute(&input.attrs)?;
        let name = input.ident.clone();
        let fields = data.fields.iter().map(MetricsField::parse);
        let fields = fields.collect::<syn::Result<_>>()?;
        Ok(Self {
            attrs,
            name,
            fields,
        })
    }

    fn initialize(&self) -> proc_macro2::TokenStream {
        let fields = self.fields.iter().map(|field| {
            let cr = self.attrs.path_to_crate(field.ty.span());
            field.initialize_default(&cr)
        });

        quote! {
            Self {
                #(#fields,)*
            }
        }
    }

    fn validate(&self) -> proc_macro2::TokenStream {
        let prefix_assertion = self.attrs.prefix.as_ref().map(|prefix| {
            let span = prefix.span();
            let cr = self.attrs.path_to_crate(span);
            quote_spanned!(span=> #cr::validation::assert_metric_prefix(#prefix);)
        });
        let field_assertions = self.fields.iter().map(|field| {
            let field_ty = &field.ty;
            let span = field_ty.span();
            let cr = self.attrs.path_to_crate(span);
            let type_assertion = quote_spanned! {span=>
                { struct _AssertIsMetric where #field_ty: #cr::BuildMetric; }
            };

            let field_name = LitStr::new(&field.name.to_string(), field.name.span());
            let span = field_name.span();
            let cr = self.attrs.path_to_crate(span);
            let name_assertion =
                quote_spanned!(span=> #cr::validation::assert_metric_name(#field_name););
            quote!(#type_assertion #name_assertion)
        });
        let label_assertions = self.fields.iter().filter_map(|field| {
            let labels = field.attrs.labels.as_ref()?;
            let span = labels.span();
            let cr = self.attrs.path_to_crate(span);
            Some(quote_spanned!(span=> #cr::validation::assert_label_names(&#labels);))
        });

        quote! {
            const _: () = {
                #prefix_assertion
                #(#field_assertions)*
                #(#label_assertions)*
            };
        }
    }

    fn implement_metrics(&self) -> proc_macro2::TokenStream {
        let name = &self.name;
        let cr = self.attrs.path_to_crate(name.span());
        let prefix = self
            .attrs
            .prefix
            .as_ref()
            .map_or_else(String::new, LitStr::value);
        let prefix = (!prefix.is_empty()).then_some(prefix.as_str());
        let visit_fields = self.fields.iter().map(|field| field.visit(prefix));
        let describe_fields = self.fields.iter().map(|field| field.describe(prefix, &cr));

        let descriptor = quote_spanned! {name.span()=>
            #cr::descriptors::MetricGroupDescriptor {
                crate_name: core::env!("CARGO_CRATE_NAME"),
                crate_version: core::env!("CARGO_PKG_VERSION"),
                module_path: core::module_path!(),
                name: core::stringify!(#name),
                line: core::line!(),
                metrics: &[#(#describe_fields,)*],
            }
        };

        quote! {
            impl #cr::Metrics for #name {
                const DESCRIPTOR: #cr::descriptors::MetricGroupDescriptor = #descriptor;

                fn visit_metrics(&self, visitor: &mut dyn #cr::MetricsVisitor) {
                    #(#visit_fields;)*
                }
            }
        }
    }

    fn derive_traits(&self) -> proc_macro2::TokenStream {
        let name = &self.name;
        let validation = self.validate();
        let initialization = self.initialize();
        let default_impl = quote! {
            impl core::default::Default for #name {
                fn default() -> Self {
                    #initialization
                }
            }
        };
        let metrics_impl = self.implement_metrics();

        quote! {
            #validation
            #default_impl
            #metrics_impl
        }
    }
}

pub(crate) fn impl_metrics(input: TokenStream) -> TokenStream {
    let input: DeriveInput = syn::parse(input).unwrap();
    let trait_impl = match MetricsImpl::new(&input) {
        Ok(trait_impl) => trait_impl,
        Err(err) => return err.into_compile_error().into(),
    };
    trait_impl.derive_traits().into()
}
