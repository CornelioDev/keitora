# Changelog

All notable changes to this project are documented in this file. [release-please](https://github.com/googleapis/release-please) generates it from Conventional Commits when a release is prepared, so don't edit it by hand ([CONTRIBUTING.md](CONTRIBUTING.md#releases)). This project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## 0.1.0-beta.1 (2026-11-14)


### Added

* **core:** add the app rule matcher (T-1.5) ([#43](https://github.com/CornelioDev/keitora/issues/43)) ([758842c](https://github.com/CornelioDev/keitora/commit/758842ce932fb302a3f52a10d6fd3bb40ec4db31))
* **core:** add the blocklist data model (T-1.1) ([#22](https://github.com/CornelioDev/keitora/issues/22)) ([859c1fa](https://github.com/CornelioDev/keitora/commit/859c1fa2aae857c22dc791fdf1f54294310741c1))
* **core:** add the clock trait and a fake clock (T-1.2) ([#23](https://github.com/CornelioDev/keitora/issues/23)) ([8aeda02](https://github.com/CornelioDev/keitora/commit/8aeda027f37a024034d1a8d9f4c083ede4fcf9da))
* **core:** add the engine snapshot (T-1.7) ([#46](https://github.com/CornelioDev/keitora/issues/46)) ([7c2e5a0](https://github.com/CornelioDev/keitora/commit/7c2e5a0c5bed03cab521b3764c5df0b6d43fddc1))
* **core:** add the pomodoro state machine (T-1.3) ([#44](https://github.com/CornelioDev/keitora/issues/44)) ([03ee466](https://github.com/CornelioDev/keitora/commit/03ee466f786346a5639b8c451a7dff38153be348))
* **core:** add the site rule matcher (T-1.4) ([#42](https://github.com/CornelioDev/keitora/issues/42)) ([067838b](https://github.com/CornelioDev/keitora/commit/067838bc6373d1b31bb3a70d53c4f2f3f52f4087))
* **core:** add the strict-mode policy (T-1.6) ([#45](https://github.com/CornelioDev/keitora/issues/45)) ([e138de2](https://github.com/CornelioDev/keitora/commit/e138de2032820c46d81b9fc1fd6d06bfd59ad0e3))
* **desktop:** add Tailwind CSS v4, shadcn-vue and a component gallery (T-0.8) ([105a9f1](https://github.com/CornelioDev/keitora/commit/105a9f1bf1b5a72a0997a9e81b57e58d4a013c0d))
* **desktop:** add the data layer (T-3.2) ([#137](https://github.com/CornelioDev/keitora/issues/137)) ([3d6661f](https://github.com/CornelioDev/keitora/commit/3d6661f3114f0f9dbde74860de978c639b263562))
* **desktop:** apply the design tokens and bundle the Geist fonts (T-0.9) ([ac3cba5](https://github.com/CornelioDev/keitora/commit/ac3cba5e707d08bfb2c9ba9a592044d74a9404a7))
* **desktop:** scaffold the Tauri 2 desktop app (T-0.6) ([2ca4091](https://github.com/CornelioDev/keitora/commit/2ca4091bb209e55785da0cc6e3b9fe7a94485ecd))
* **desktop:** set up vue-i18n in English and Spanish (T-0.10) ([5dbf87f](https://github.com/CornelioDev/keitora/commit/5dbf87fa35ff1ac44cd687c84b5264cc26b3d2dc))
* **extension:** add Manifest V3 skeleton with Vite, ESLint, Prettier and Vitest (T-0.11) ([c5f0e4b](https://github.com/CornelioDev/keitora/commit/c5f0e4b1d55dee86256d77883a9d13daeceb5f0e))
* **ipc:** add the versioned IPC protocol (T-2.3) ([#131](https://github.com/CornelioDev/keitora/issues/131)) ([765eb9b](https://github.com/CornelioDev/keitora/commit/765eb9bd71f5ae7d87b888ce070b865f6be622ad))
* **service:** add SQLite persistence with migrations (T-2.2) ([#54](https://github.com/CornelioDev/keitora/issues/54)) ([fd9ec21](https://github.com/CornelioDev/keitora/commit/fd9ec214b8a6c4d21fc4a460a9efca39d621ed3b))
* **service:** add the named-pipe server (T-2.4) ([#136](https://github.com/CornelioDev/keitora/issues/136)) ([bf629df](https://github.com/CornelioDev/keitora/commit/bf629df9f90517a5d534da40ac5ea6b512e32f86))
* **service:** add the service binary, console mode and daily logs (T-2.1) ([#51](https://github.com/CornelioDev/keitora/issues/51)) ([5e37a22](https://github.com/CornelioDev/keitora/commit/5e37a22dcf282895c677d874c19d610cdc9aea4b))


### Fixed

* **tooling:** make run-on-staged work in linked worktrees ([#132](https://github.com/CornelioDev/keitora/issues/132)) ([8c94b5e](https://github.com/CornelioDev/keitora/commit/8c94b5e916887900800000738f6da6533e496892))

## [Before the first release]

Written by hand before releases were automated. The first release's entry lists part of this work again, by commit.

### Added

- Project plan, architecture, threat model and ADRs 0001–0008.
- Agent-based development workflow with QA, developer and security subagents.
- Desktop app skeleton (Tauri 2 + Vue 3) named Keitora: an empty window with a strict Content Security Policy and no Tauri permissions.
- Desktop app design: the color tokens, a harmonized corner radius scale, and the Geist and Geist Mono fonts bundled with the app.
- Browser extension skeleton (Manifest V3) for Chrome and Edge, named Keitora and localized in English and Spanish. It requests no permissions and doesn't block anything yet.
