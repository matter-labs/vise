# Changelog

All notable changes to this project will be documented in this file.
The project adheres to [Semantic Versioning](http://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Allow sharing common label set among multiple metrics using an interface similar to `Family` (#30).
- Implement lazy getter for `Family` / `MetricsFamily` (#32).

### Changed

- Bump minimum supported Rust version to 1.79 (#30).
- **exporter:** Make `MetricsExporter::bind()` method async (#31).
- Change `Family::to_entries()` to return an iterator (#32).

### Removed

- Remove legacy metrics support (#31).

## 0.2.0 - 2024-08-07

### Changed

- Bump minimum supported Rust version to 1.70 (#26).

### Fixed

- Use [`ctor`](https://crates.io/crates/ctor) to handle metrics registration instead of [`linkme`](https://crates.io/crates/linkme).
  `linkme` requires linker configuration on recent nightly Rust versions, which degrades DevEx. (#26)

## 0.1.0 - 2024-07-05

The initial release of `vise`.
