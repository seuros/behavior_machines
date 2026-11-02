# Changelog

## [0.4.0](https://github.com/seuros/behavior_machines/compare/behavior-machines-circuit-v0.3.1...behavior-machines-circuit-v0.4.0) (2026-11-02)


### ⚠ BREAKING CHANGES

* **circuit:** trees, statuses and times come from behavior-machines 0.3; a Node built with behavior-machines 0.2 is a different type.

### Bug Fixes

* **circuit:** lower trees from behavior-machines 0.3 ([5f8d8ee](https://github.com/seuros/behavior_machines/commit/5f8d8eefaa6884250a14ecbaa72d8c26361f724a))


### Dependencies

* The following workspace dependencies were updated
  * dependencies
    * behavior-machines bumped from 0.2.0 to 0.3.0

## [0.3.1](https://github.com/seuros/behavior_machines/compare/behavior-machines-circuit-v0.3.0...behavior-machines-circuit-v0.3.1) (2026-10-29)


### Bug Fixes

* **circuit:** reject cursor state from another tree; drop dead halt ops ([6a445b1](https://github.com/seuros/behavior_machines/commit/6a445b1635c736e245a653591fbc3af895613ed5))

## [0.3.0](https://github.com/seuros/behavior_machines/compare/behavior-machines-circuit-v0.1.0...behavior-machines-circuit-v0.3.0) (2026-10-28)


### Features

* add behavior-machines-circuit crate ([d066365](https://github.com/seuros/behavior_machines/commit/d066365048972d49ab41a09d668e62e3b9aaa407))
* **circuit:** stage effectful actions with explicit cancellation ([e207c2e](https://github.com/seuros/behavior_machines/commit/e207c2e880fb858a02dd9b0270af9ae8d033a239))
