//! Metric exporter based on the `hyper` web server / client.
//!
//! An exporter scrapes metrics from a [`Registry`] and allows exporting them to Prometheus by either
//! running a web server or pushing to the Prometheus push gateway. An exporter should only be initialized
//! in applications, not libraries.
//!
//! # Crate features
//!
//! ## `legacy`
//!
//! *(Off by default)*
//!
//! Enables exporting metrics defined with the `metrics` façade, in addition to those defined
//! using `vise`.
//!
//! # Examples
//!
//! Running a pull-based exporter with graceful shutdown:
//!
//! ```
//! use tokio::sync::watch;
//! use vise_exporter::MetricsExporter;
//!
//! async fn my_app() {
//!     let (shutdown_sender, mut shutdown_receiver) = watch::channel(());
//!     let exporter = MetricsExporter::default()
//!         .with_graceful_shutdown(async move {
//!             shutdown_receiver.changed().await.ok();
//!         });
//!     let bind_address = "0.0.0.0:3312".parse().unwrap();
//!     tokio::spawn(exporter.start(bind_address));
//!
//!     // Then, once the app is shutting down:
//!     shutdown_sender.send_replace(());
//! }
//! ```
//!
//! Running a push-based exporter that scrapes metrics each 10 seconds:
//!
//! ```
//! # use std::time::Duration;
//! # use tokio::sync::watch;
//! # use vise_exporter::MetricsExporter;
//! async fn my_app() {
//!     let exporter = MetricsExporter::default();
//!     let exporter_task = exporter.push_to_gateway(
//!         "http://prom-gateway/job/pushgateway/instance/my_app".parse().unwrap(),
//!         Duration::from_secs(10),
//!     );
//!     tokio::spawn(exporter_task);
//! }
//! ```

// Documentation settings.
#![doc(html_root_url = "https://docs.rs/vise-exporter/0.1.0")]
#![cfg_attr(docsrs, feature(doc_cfg))]
// Linter settings.
#![warn(missing_debug_implementations, missing_docs, bare_trait_objects)]
#![warn(clippy::all, clippy::pedantic)]
#![allow(clippy::must_use_candidate, clippy::module_name_repetitions)]

// Reexport to simplify configuring legacy exporter.
#[cfg(feature = "legacy")]
pub use metrics_exporter_prometheus;

mod exporter;
mod metrics;

pub use crate::exporter::{MetricsExporter, MetricsServer};

#[cfg(doctest)]
doc_comment::doctest!("../README.md");
