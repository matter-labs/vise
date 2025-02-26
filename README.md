# Vise – Typesafe Metrics Client and Exporter

[![Build Status](https://github.com/matter-labs/vise/workflows/Rust/badge.svg?branch=main)](https://github.com/matter-labs/vise/actions)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/License-MIT%2FApache--2.0-blue)](https://github.com/matter-labs/vise#license)
![rust 1.79+ required](https://img.shields.io/badge/rust-1.79+-blue.svg?label=Required%20Rust)

This repository provides a collection of tools to define and export metrics in Rust
libraries and applications.

## Overview

The following crates are included:

- [`vise`](crates/vise) is the client library for typesafe metrics definition
- [`vise-macros`](crates/vise-macros) is a collection of procedural macros used by `vise`
- [`vise-exporter`](crates/vise-exporter) is a Prometheus exporter for `vise` metrics
  supporting pull- and push-based data flows.

Follow the [client library readme](crates/vise/README.md) for an overview of functionality.

## Naming

[Vise](https://en.wikipedia.org/wiki/Vise) is a mechanical tool used to secure an object in place,
for example to perform precise measurements on it.

## License

Distributed under the terms of either

- Apache License, Version 2.0, ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option.
