# Changelog

## [0.11.0](https://github.com/special-place-ai-heaven/coding-supervisor/compare/v0.10.1...v0.11.0) (2026-11-05)


### Features

* **gate:** bind a task to a linked worktree (G5) and keep evidence across a tree-identical commit (G9) ([0f3b40d](https://github.com/special-place-ai-heaven/coding-supervisor/commit/0f3b40d5e27aaeac79d0c5d60f51c9c36b245c0a))
* **gate:** bind a task to a linked worktree and keep evidence across a tree-identical commit ([0bd8aa8](https://github.com/special-place-ai-heaven/coding-supervisor/commit/0bd8aa8b47a22f95c83fffce416a4799fb24e2b3))


### Bug Fixes

* **gate:** keep worktree lookups off git; canonicalize unborn paths ([c4b0cbd](https://github.com/special-place-ai-heaven/coding-supervisor/commit/c4b0cbdf9cdf3934cae50ff2226cdf34cca32b30))

## [0.10.1](https://github.com/special-place-ai-heaven/coding-supervisor/compare/v0.10.0...v0.10.1) (2026-11-05)


### Bug Fixes

* **gate:** keep read-only commands open after completion and end stopped jobs ([fe43487](https://github.com/special-place-ai-heaven/coding-supervisor/commit/fe4348757af96ec9eda8fa47df0eb91dc8e7653d))
* **gate:** keep read-only commands open after completion and end stopped jobs ([b81b86a](https://github.com/special-place-ai-heaven/coding-supervisor/commit/b81b86a3f016c41b4f1dbed1e7034e767b2b9983))

## [0.10.0](https://github.com/special-place-ai-heaven/coding-supervisor/compare/v0.9.3...v0.10.0) (2026-11-05)


### Features

* **gate:** refuse pushes to the default branch, force-pushes and npm publish by default ([#46](https://github.com/special-place-ai-heaven/coding-supervisor/issues/46)) ([c2bc72f](https://github.com/special-place-ai-heaven/coding-supervisor/commit/c2bc72fd01cebe2a08889743359454e9aaddfb62))

## [0.9.3](https://github.com/special-place-ai-heaven/coding-supervisor/compare/v0.9.2...v0.9.3) (2026-11-05)


### Bug Fixes

* **gate:** close the linked-worktree gap and name the way out of two refusals ([4a87208](https://github.com/special-place-ai-heaven/coding-supervisor/commit/4a8720876d4a3147c7d48ba5e13f02417cb0f322))
* **gate:** close the linked-worktree gap and name the way out of two refusals ([bf49e42](https://github.com/special-place-ai-heaven/coding-supervisor/commit/bf49e420b7dbeded9e5270d78d356a609e2d0196))

## [0.9.2](https://github.com/special-place-ai-heaven/coding-supervisor/compare/v0.9.1...v0.9.2) (2026-11-05)


### Bug Fixes

* **gate:** stop dead-ending agents on discussion, foreign tasks and bad --project ([e927dff](https://github.com/special-place-ai-heaven/coding-supervisor/commit/e927dff6459f69d4db64f09df388158d402d0f35))
* **gate:** stop dead-ending agents on discussion, foreign tasks and bad --project ([279471f](https://github.com/special-place-ai-heaven/coding-supervisor/commit/279471fab948dd4ec3e5f1e05655a771956024c4))

## [0.9.1](https://github.com/special-place-ai-heaven/coding-supervisor/compare/v0.9.0...v0.9.1) (2026-11-04)


### Bug Fixes

* **autonomy:** roll over a completed task after its check was rerun ([6969873](https://github.com/special-place-ai-heaven/coding-supervisor/commit/696987358b49d30ec5d6418a5b7208f98e05d583))
* **autonomy:** roll over a completed task after its check was rerun ([a58ab57](https://github.com/special-place-ai-heaven/coding-supervisor/commit/a58ab571ffcd05c75082609b61de24906fb01b37))

## [0.9.0](https://github.com/special-place-ai-heaven/coding-supervisor/compare/v0.8.1...v0.9.0) (2026-11-03)


### Features

* **cli:** let a person cancel a stuck task without a session ([7390fd1](https://github.com/special-place-ai-heaven/coding-supervisor/commit/7390fd19a4372e801d6f9a681c4f5b853dbd22c6))
* **cli:** let a person cancel a stuck task without a session ([2ab11e7](https://github.com/special-place-ai-heaven/coding-supervisor/commit/2ab11e71859b81db89c155b618a38b8bfbb71863))

## [0.8.1](https://github.com/special-place-ai-heaven/coding-supervisor/compare/v0.8.0...v0.8.1) (2026-11-03)


### Bug Fixes

* **claude-code:** check a remembered worktree root before using it ([45d429d](https://github.com/special-place-ai-heaven/coding-supervisor/commit/45d429d0b0c0ff4e1334ba19a694b9084641e247))


### Performance Improvements

* **claude-code:** start 3 git processes per hook call instead of up to 19 ([0464b10](https://github.com/special-place-ai-heaven/coding-supervisor/commit/0464b10435f9bd84bf17cd32b1f1f23077b66fc0))
* start 3 git processes per hook call instead of up to 19 ([748e505](https://github.com/special-place-ai-heaven/coding-supervisor/commit/748e50592eaff1972afbda3259ed38d1dc35d992))

## [0.8.0](https://github.com/special-place-ai-heaven/coding-supervisor/compare/v0.7.0...v0.8.0) (2026-11-03)


### Features

* what strict mode adds ([1e65cb0](https://github.com/special-place-ai-heaven/coding-supervisor/commit/1e65cb07baade71c297325ccd1cdebaf9ab1f98b))

## [0.7.0](https://github.com/special-place-ai-heaven/coding-supervisor/compare/v0.6.0...v0.7.0) (2026-11-03)


### Features

* gate modes off, audit, enforce and strict ([9cafc23](https://github.com/special-place-ai-heaven/coding-supervisor/commit/9cafc23794170648387385b79d6d4f88ecbed69c))

## [0.6.0](https://github.com/special-place-ai-heaven/coding-supervisor/compare/v0.5.5...v0.6.0) (2026-11-03)


### Features

* **cli:** version flag, discovered init draft, clearer verdict and status ([55dce75](https://github.com/special-place-ai-heaven/coding-supervisor/commit/55dce7570b26c7e994aee9e1ed2a571b35ca3d43))
* **npm:** add coding-supervisor as a second command name ([ad4e1c9](https://github.com/special-place-ai-heaven/coding-supervisor/commit/ad4e1c968bc25a045925f924697c16a9f9abd4f5))


### Bug Fixes

* **claude-code:** classify a prompt from the shell when the MCP server is absent ([eeb713b](https://github.com/special-place-ai-heaven/coding-supervisor/commit/eeb713bdccaa604dddadca4d632638d654e58bdb))
* **claude-code:** journal early refusals and warn about unenforced effects ([949b768](https://github.com/special-place-ai-heaven/coding-supervisor/commit/949b768aca7539864d1c2f4eae01b6001f55ef51))
* **claude-code:** let a person start a task after prompts were registered ([f033bd6](https://github.com/special-place-ai-heaven/coding-supervisor/commit/f033bd62911cd4bc85051b7b6595d0b81c731989))
* **claude-code:** record checks whose quoted pattern contains a pipe ([9d33a6a](https://github.com/special-place-ai-heaven/coding-supervisor/commit/9d33a6ac186d6b6684645d198fee7c8eee99f730))
* field follow-ups from 0.5.5 and the dogfood lock-out ([68ad718](https://github.com/special-place-ai-heaven/coding-supervisor/commit/68ad718cfe7a921256be11330c42efd039b7728b))
* **install:** follow CODEX_HOME and name files git tracks ([0f9682d](https://github.com/special-place-ai-heaven/coding-supervisor/commit/0f9682d16a1bff207861424adc9a309dd0dc368b))

## [0.5.5](https://github.com/special-place-ai-heaven/coding-supervisor/compare/v0.5.4...v0.5.5) (2026-11-02)


### Bug Fixes

* **claude-code:** discover npm checks in direct subfolders and let a person amend them ([cd12ed4](https://github.com/special-place-ai-heaven/coding-supervisor/commit/cd12ed442266ba32714f4e47863b21a99fd374f8))
* **claude-code:** discover npm checks in direct subfolders and let a person amend them ([cca6f62](https://github.com/special-place-ai-heaven/coding-supervisor/commit/cca6f6240bc6f0d3efe6d1be038ebc32e3d97bdc))
* **claude-code:** keep one floor entry per check id and name every differing field ([a5fbc3a](https://github.com/special-place-ai-heaven/coding-supervisor/commit/a5fbc3ab20e93e33b4fe16e207f2297f92fadc07))

## [0.5.4](https://github.com/special-place-ai-heaven/coding-supervisor/compare/v0.5.3...v0.5.4) (2026-11-02)


### Bug Fixes

* **claude-code:** charge at most one repair round per version of the code ([be04527](https://github.com/special-place-ai-heaven/coding-supervisor/commit/be04527188f5d1e1554eed442c6e7a41fcd9385e))
* **claude-code:** charge at most one repair round per version of the code ([06d6b4a](https://github.com/special-place-ai-heaven/coding-supervisor/commit/06d6b4a55a89ae87588c4b13130232b4beca7766))

## [0.5.3](https://github.com/special-place-ai-heaven/coding-supervisor/compare/v0.5.2...v0.5.3) (2026-11-02)


### Bug Fixes

* **claude-code:** spend repair rounds only on a failing candidate, add operator escapes ([76eac21](https://github.com/special-place-ai-heaven/coding-supervisor/commit/76eac219d53c1a2ae44f9ab5af6dc55134c9744a))
* **claude-code:** spend repair rounds only on a failing candidate, add operator escapes ([70226b9](https://github.com/special-place-ai-heaven/coding-supervisor/commit/70226b9c0d9787e86fa4c9fe322551751d93a5cb))

## [0.5.2](https://github.com/special-place-ai-heaven/coding-supervisor/compare/v0.5.1...v0.5.2) (2026-11-02)


### Bug Fixes

* **claude-code:** let an agent classify its prompt without being trapped ([5d1cfeb](https://github.com/special-place-ai-heaven/coding-supervisor/commit/5d1cfeba98eaac8f1d1627de653e6a970d3bdfa6))
* **claude-code:** let an agent classify its prompt without being trapped ([e8132b3](https://github.com/special-place-ai-heaven/coding-supervisor/commit/e8132b38cc2ab83ec941ce2fac37895964c0f984))

## [0.5.1](https://github.com/special-place-ai-heaven/coding-supervisor/compare/v0.5.0...v0.5.1) (2026-11-01)


### Bug Fixes

* bound git and state.lock waits on the hook path ([43782b8](https://github.com/special-place-ai-heaven/coding-supervisor/commit/43782b8d776d7c67bb38208d32e68e7dc867bcf0))

## [0.5.0](https://github.com/special-place-ai-heaven/coding-supervisor/compare/v0.4.1...v0.5.0) (2026-11-01)


### Features

* **plugin:** add Claude Code plugin with fail-closed supervisor hook ([21bf34f](https://github.com/special-place-ai-heaven/coding-supervisor/commit/21bf34fc20cfbc7668dcb9bbec5402614dcb52cc))

## [0.4.1](https://github.com/special-place-ai-heaven/coding-supervisor/compare/v0.4.0...v0.4.1) (2026-11-01)


### Bug Fixes

* **level:** stamp effective level, document precedence, close KENT test gaps ([155b6b0](https://github.com/special-place-ai-heaven/coding-supervisor/commit/155b6b02fe51ec3efad0ce8da33d8777e1766619))
* **level:** stamp effective level, document precedence, close KENT test gaps ([7380255](https://github.com/special-place-ai-heaven/coding-supervisor/commit/738025597b7e36caf19b3c67cf23a9866ef7dbfa))
* **level:** unreadable level source, status level_source, prove off certifies nothing ([eaf58e3](https://github.com/special-place-ai-heaven/coding-supervisor/commit/eaf58e3eea79e42a9a89a2b647c8d35db40dbaac))

## [0.4.0](https://github.com/special-place-ai-heaven/coding-supervisor/compare/v0.3.0...v0.4.0) (2026-11-01)


### Features

* **level:** add supervisor level off|lite|full|ultra ([2f3777a](https://github.com/special-place-ai-heaven/coding-supervisor/commit/2f3777a3518b0c641ca1895eacc651f0fd9ef882))
* **level:** add supervisor level off|lite|full|ultra ([8c3177b](https://github.com/special-place-ai-heaven/coding-supervisor/commit/8c3177b3130591a4771d8291533b025d4a90259f))


### Bug Fixes

* **level:** resolve the deepest existing ancestor before the ultra scope check ([a75212b](https://github.com/special-place-ai-heaven/coding-supervisor/commit/a75212bfc74da34b34be20a5ca0606674590191b))

## [0.3.0](https://github.com/special-place-ai-heaven/coding-supervisor/compare/v0.2.1...v0.3.0) (2026-11-01)


### Features

* **verdict:** add supervisor verdict --require observed|asserted ([05a221e](https://github.com/special-place-ai-heaven/coding-supervisor/commit/05a221e550344559e3a5f06070c07cf2f8676ca7))
* **verdict:** add supervisor verdict --require observed|asserted ([f4ee24f](https://github.com/special-place-ai-heaven/coding-supervisor/commit/f4ee24f6865b993c50ea63be4dc3cd5c5757fa59))
* **verdict:** conform verdict to contract r3 ([1234161](https://github.com/special-place-ai-heaven/coding-supervisor/commit/12341612095152215542a94eb34a7aaefbdcd2e4))

## [0.2.1](https://github.com/special-place-ai-heaven/coding-supervisor/compare/v0.2.0...v0.2.1) (2026-10-30)


### Bug Fixes

* make recorded receipt display agree with acceptance ([28b3238](https://github.com/special-place-ai-heaven/coding-supervisor/commit/28b32386faa23bbfd022208db3bc72376bf0989d))

## [0.2.0](https://github.com/special-place-ai-heaven/coding-supervisor/compare/v0.1.1...v0.2.0) (2026-10-30)


### Features

* automate supervised tasks and repair evidence isolation ([e71d6d4](https://github.com/special-place-ai-heaven/coding-supervisor/commit/e71d6d4e11e14c089e32e1009c37c0b33815a6e6))


### Bug Fixes

* journal exit codes for terminal-commander failed jobs ([8279c3b](https://github.com/special-place-ai-heaven/coding-supervisor/commit/8279c3bbea24817673529b8a87a1ba57396409d3))
* report recorded receipts after a terminal resume ([57d2c6e](https://github.com/special-place-ai-heaven/coding-supervisor/commit/57d2c6e0301eecf520da793449713a8d7b5a8edc))
* run autonomy fixture scripts with python3 on unix ([9846bfb](https://github.com/special-place-ai-heaven/coding-supervisor/commit/9846bfb3bd482f3fe3427bc80d2fec6677bbdfb0))
* run execution-context fixture scripts with python3 on unix ([70e8cfd](https://github.com/special-place-ai-heaven/coding-supervisor/commit/70e8cfdeb82ed1ef4b43caf2e0f2783ffcc3479a))

## [0.1.1](https://github.com/special-place-ai-heaven/coding-supervisor/compare/v0.1.0...v0.1.1) (2026-10-30)


### Bug Fixes

* bind completion evidence to explicit task lifecycle ([6e295b2](https://github.com/special-place-ai-heaven/coding-supervisor/commit/6e295b22190550e373a3d64fbda5e55669d67061))

## 0.1.0 (2026-10-29)


### Bug Fixes

* **npm:** name the launcher's own version in the reinstall hint ([f78208d](https://github.com/special-place-ai-heaven/coding-supervisor/commit/f78208d2464b575c80730f727dd19341afc6113f))


### Continuous Integration

* release automatically from main with release-please ([71efdbe](https://github.com/special-place-ai-heaven/coding-supervisor/commit/71efdbe6f8fd8dbcc6066e20b93bccb12a776a72))
