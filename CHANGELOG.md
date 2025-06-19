# Changelog

All notable changes to this project will be documented in this file.
The project adheres to [Semantic Versioning](http://semver.org/spec/v2.0.0.html).

## [0.3.2](https://github.com/matter-labs/vise/compare/v0.3.1...v0.3.2) (2025-06-18)


### Features

* Allow `str` indexing for labeled families ([#39](https://github.com/matter-labs/vise/issues/39)) ([20fe082](https://github.com/matter-labs/vise/commit/20fe082d2cdf21963dd0a20213c77652251ecd2c))

## [0.3.1](https://github.com/matter-labs/vise/compare/v0.3.0...v0.3.1) (2025-05-21)


### Bug Fixes

* Escape label values ([#37](https://github.com/matter-labs/vise/issues/37)) ([8c64392](https://github.com/matter-labs/vise/commit/8c64392753ab8fe563c6fa3db5e4d987952ad011))

## [0.3.0](https://github.com/matter-labs/vise/compare/v0.2.0...v0.3.0) (2025-04-11)


### ⚠ BREAKING CHANGES

* Implement lazy getter for families ([#32](https://github.com/matter-labs/vise/issues/32))
* Update dependencies ([#31](https://github.com/matter-labs/vise/issues/31))
* Share common labels among metrics ([#30](https://github.com/matter-labs/vise/issues/30))

### Features

* Implement lazy getter for families ([#32](https://github.com/matter-labs/vise/issues/32)) ([51669f4](https://github.com/matter-labs/vise/commit/51669f42f60c50b3a521662a4ecd71212a303299))
* Share common labels among metrics ([#30](https://github.com/matter-labs/vise/issues/30)) ([6c010a6](https://github.com/matter-labs/vise/commit/6c010a683de9750fb61ca2d30630d79afbe743d8))


### Bug Fixes

* add release-please config and workflows ([#35](https://github.com/matter-labs/vise/issues/35)) ([5c5dc01](https://github.com/matter-labs/vise/commit/5c5dc01aac57a2695b57e4574b6a885e5bfa1107))


### Miscellaneous Chores

* Update dependencies ([#31](https://github.com/matter-labs/vise/issues/31)) ([2ce1d69](https://github.com/matter-labs/vise/commit/2ce1d69a12a011fa1f9dfe10f50393cb3203c24d))

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
