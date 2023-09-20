//! Procedural macros for the `vise` metrics client.
//!
//! All macros in this crate are re-exported from the [`vise`] crate. See its docs for more details
//! and the examples of usage.
//!
//! [`vise`]: https://docs.rs/vise/

// Documentation settings.
#![doc(html_root_url = "https://docs.rs/vise-macros/0.1.0")]
// General settings.
#![recursion_limit = "128"]
// Linter settings.
#![warn(missing_debug_implementations, bare_trait_objects)]
#![warn(clippy::all, clippy::pedantic)]
#![allow(clippy::must_use_candidate, clippy::module_name_repetitions)]

extern crate proc_macro;

use proc_macro::TokenStream;

mod labels;
mod metrics;
mod register;
mod utils;

#[proc_macro_derive(Metrics, attributes(metrics))]
pub fn metrics(input: TokenStream) -> TokenStream {
    metrics::impl_metrics(input)
}

#[proc_macro_derive(EncodeLabelValue, attributes(metrics))]
pub fn encode_label_value(input: TokenStream) -> TokenStream {
    labels::impl_encode_label_value(input)
}

#[proc_macro_derive(EncodeLabelSet, attributes(metrics))]
pub fn encode_label_set(input: TokenStream) -> TokenStream {
    labels::impl_encode_label_set(input)
}

#[proc_macro_attribute]
pub fn register(_attrs: TokenStream, input: TokenStream) -> TokenStream {
    register::impl_register(input)
}
