# Changelog

All notable changes to this project will be documented in this file.
The project adheres to [Semantic Versioning](http://semver.org/spec/v2.0.0.html).

## [Unreleased]

## 0.1.1 - 2024-08-07

### Changed

- Bump minimum supported Rust version to 1.70 (#26).

### Fixed

- Use [`ctor`](https://crates.io/crates/ctor) to handle metrics registration instead of [`linkme`](https://crates.io/crates/linkme).
  `linkme` requires linker configuration on recent nightly Rust versions, which degrades DevEx. (#26)

## 0.1.0 - 2024-07-05

The initial release of `vise`.
