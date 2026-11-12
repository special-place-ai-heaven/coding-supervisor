# Changelog

## [0.21.0](https://github.com/special-place-ai-heaven/coding-supervisor/compare/v0.20.1...v0.21.0) (2026-11-12)


### Features

* name the hook-observed receipts a completed task was accepted on in the next prompt's context ([a40fa53](https://github.com/special-place-ai-heaven/coding-supervisor/commit/a40fa5347bb3b75e1001c5eeba6aa79a89ecacbe))


### Bug Fixes

* do not count the pending hold and clear it at every prompt of the task's session ([cfbcca9](https://github.com/special-place-ai-heaven/coding-supervisor/commit/cfbcca91c34ec472c45a18f282b25fd0eb017a23))
* do not number a stop held only for a job with no recorded exit, and release the continued stop ([ef09e64](https://github.com/special-place-ai-heaven/coding-supervisor/commit/ef09e64b2594db143f032845f0c42825d07f6442))
* keep a check run when a poll cannot read the code version, and stop numbering a pending-job hold ([d52d3e8](https://github.com/special-place-ai-heaven/coding-supervisor/commit/d52d3e86df5fe2001036c38c19e0735f61eaa8c1))
* keep a Terminal Commander run when a poll cannot read the code version, and name a cancelled run ([36bde8d](https://github.com/special-place-ai-heaven/coding-supervisor/commit/36bde8dc6a8df1f47addaac0ec4c7b387df3095c))
* name a cancelled run only on its own cancellation row and qualify the hold's release note ([076a22d](https://github.com/special-place-ai-heaven/coding-supervisor/commit/076a22ddf8bd670619aa917e6fa7fec5b2783117))
* release a continued pending-only stop only right after the gate's own pending hold ([82b721f](https://github.com/special-place-ai-heaven/coding-supervisor/commit/82b721f86cc22e0c17945b00be4ffb668697ae3e))

## [0.20.1](https://github.com/special-place-ai-heaven/coding-supervisor/compare/v0.20.0...v0.20.1) (2026-11-12)


### Bug Fixes

* approve thread-count, colour and output-level variables on check runs ([9ae8356](https://github.com/special-place-ai-heaven/coding-supervisor/commit/9ae83566977c27f1bdab278cc91b8f927a1bf269))
* compare only this session's own intent entries before a judged hook call ([d4687cd](https://github.com/special-place-ai-heaven/coding-supervisor/commit/d4687cda860d7a8123b55d1a45e42971048ce2f1))
* count a comma anywhere after a brace as expanding in the read-only shell check ([4a58dab](https://github.com/special-place-ai-heaven/coding-supervisor/commit/4a58dabf0a2dbc408943d6f97ae33f8b97ea6209))
* give a stale session of a caller-asserted task advice it can follow ([19a0c4e](https://github.com/special-place-ai-heaven/coding-supervisor/commit/19a0c4e2e286d1dfaa32fe9292ca145ba0008ae4))
* judge shell expansion as Bash does and take only ./ directories after cd ([e791a7f](https://github.com/special-place-ai-heaven/coding-supervisor/commit/e791a7f4a863d2d637a87dfdf0eb850d10b92169))
* keep a pre-intent cd inside the same project and refuse expanded words that could become --output ([837d87a](https://github.com/special-place-ai-heaven/coding-supervisor/commit/837d87ab1e5fa9d3ff8528fde26743e9863f1a14))
* keep task cleanup from deleting marked files it could not look up and stop offering rebind for prompt-managed tasks ([daa989b](https://github.com/special-place-ai-heaven/coding-supervisor/commit/daa989b1e49b75f334724d2c16a6e63b1f1aae1f))
* let read-only git, gh, cat and which commands run before an intent and say how to work in parallel ([c5dcea8](https://github.com/special-place-ai-heaven/coding-supervisor/commit/c5dcea82413b4fc9e73d9ce2ada4718dc01b4c27))
* pin the quoted --output refusal, keep gh api placeholders and state the hooks' refusals by level ([4064514](https://github.com/special-place-ai-heaven/coding-supervisor/commit/406451472aa1a215cae2aadf8e1d0ac02ef8b1b3))
* read a backslash inside double quotes as Bash does in the read-only shell check ([d531650](https://github.com/special-place-ai-heaven/coding-supervisor/commit/d53165050e7c5f8357017926362043f3de9136dd))
* read double quotes and only space or tab as word breaks in the read-only shell check ([7230c2a](https://github.com/special-place-ai-heaven/coding-supervisor/commit/7230c2a76a8bacc9243f315300f47332496718e0))
* read pre-task shell commands as Bash does and allow more read-only ones ([b41e494](https://github.com/special-place-ai-heaven/coding-supervisor/commit/b41e4945d7979a385f209e190789b99e6b64853e))
* refuse a pre-intent cd into a directory word starting with # ([5880744](https://github.com/special-place-ai-heaven/coding-supervisor/commit/58807445f33b28708491bbee77f424eb70b0fdbc))
* refuse a pre-intent cd whose directory word holds a backslash or double quote ([bb66fa0](https://github.com/special-place-ai-heaven/coding-supervisor/commit/bb66fa01515ab1adf4e2eba87a67026f97760861))
* refuse a pre-intent cd with a .. part or a leading backslash and state the glob limit ([1107804](https://github.com/special-place-ai-heaven/coding-supervisor/commit/11078047ed1a6d97c47ba728634de61eb12aae85))
* refuse a tab inside quotes in the read-only shell check ([b3c01b8](https://github.com/special-place-ai-heaven/coding-supervisor/commit/b3c01b8a2d7b054acd7d11ce7a25c11179d3b58d))
* refuse an expanded git -C directory and say which session can resume a caller-asserted task ([3f120a8](https://github.com/special-place-ai-heaven/coding-supervisor/commit/3f120a88e00bc3d1f5c56b790b0967d9e26ecda3))
* say that an intent_apply answer records the proposal and the hooks do the refusing ([6cf03c2](https://github.com/special-place-ai-heaven/coding-supervisor/commit/6cf03c2bbaaa18ddf151f35097213d68782d3265))

## [0.20.0](https://github.com/special-place-ai-heaven/coding-supervisor/compare/v0.19.2...v0.20.0) (2026-11-11)


### Features

* create and clean up a worktree for a person's task ([678f94d](https://github.com/special-place-ai-heaven/coding-supervisor/commit/678f94dfec3f38015c47e4277d2ed7de6ba98fda))
* create and clean up a worktree for a person's task ([ef30d5b](https://github.com/special-place-ai-heaven/coding-supervisor/commit/ef30d5b6528612753397da5a529506fc520a93f1))


### Bug Fixes

* check a worktree task's own worktree for uncommitted work before a takeover ([5de5871](https://github.com/special-place-ai-heaven/coding-supervisor/commit/5de587112757734f1315d4f29725418ead432cc5))
* find a linked-worktree start's task from its new worktree, refuse a revise that moves a worktree task ([393b166](https://github.com/special-place-ai-heaven/coding-supervisor/commit/393b1661f8a1db84c7df98ffed75dfb61a455195))
* give the full rebind command when a session belongs to an earlier task ([c6cda37](https://github.com/special-place-ai-heaven/coding-supervisor/commit/c6cda3778c964b1a49301136e5d98a4ceedbb53e))
* keep a journaled worktree start, store the base checkout, unlink nested .git links ([648a552](https://github.com/special-place-ai-heaven/coding-supervisor/commit/648a55287644be8ea8a775cdd8eff6c14c677f69))
* keep a person's task a person's after a prompt makes it prompt-managed ([e65fb3c](https://github.com/special-place-ai-heaven/coding-supervisor/commit/e65fb3c4df34ebdab0b66fcd1a9d6bb25f79a23c))
* keep the primary checkout guarded for a task started from a linked worktree ([5930424](https://github.com/special-place-ai-heaven/coding-supervisor/commit/5930424cff954e805d58cd72af8e5ffef01a95c0))
* match npm's launchers only by the package's exact bin names ([dcd9297](https://github.com/special-place-ai-heaven/coding-supervisor/commit/dcd9297273970b62d2512dadca33c2650cb0a6b1))
* name the cancel when a prompt-managed person's task cannot be replaced ([58b0bd7](https://github.com/special-place-ai-heaven/coding-supervisor/commit/58b0bd7b036125091bc22f8008e40f8800d77519))
* name the earlier worktree when a start is replayed without one ([cf21a6e](https://github.com/special-place-ai-heaven/coding-supervisor/commit/cf21a6ebebd928855871f240e6fab96ab335dd1d))
* offer only the person's cancel when a hook prompt cannot resume the task ([859c0f2](https://github.com/special-place-ai-heaven/coding-supervisor/commit/859c0f2e68599372bcabad23e63ce0200ddbe1f4))
* refuse a person's commands through npm's launchers ([b3132c0](https://github.com/special-place-ai-heaven/coding-supervisor/commit/b3132c01a5c81e0708605fe8eac89bcb7fbe1c57))
* refuse a person's commands through the npm package's launcher module ([960225b](https://github.com/special-place-ai-heaven/coding-supervisor/commit/960225b0981377589ae8a59595778e0226881bc4))
* refuse a replayed start with another worktree setting, name the stored isolation in its warning ([aa18739](https://github.com/special-place-ai-heaven/coding-supervisor/commit/aa18739f610c8032674081cdf1b389808cf121ad))
* refuse cleanup only for a marked file present in the worktree ([e9e797d](https://github.com/special-place-ai-heaven/coding-supervisor/commit/e9e797dcfb9c620b47fa4bf5f537a611a1da7789))
* refuse cleanup while a worktree file is marked assume-unchanged or skip-worktree ([22cb20d](https://github.com/special-place-ai-heaven/coding-supervisor/commit/22cb20d744e976e31a3266dbad7f7bcf89364211))
* refuse task cleanup while the worktree holds ignored files ([526afe3](https://github.com/special-place-ai-heaven/coding-supervisor/commit/526afe3eb39affe414b0687f4bfb08cd389cd4d5))
* refuse task deactivate, rebind and migrate from agents ([ee1bd90](https://github.com/special-place-ai-heaven/coding-supervisor/commit/ee1bd90349b5d66ad4d9a00acb504061bb646395))
* say "abandon_previous" does not apply to a task a person started ([130d5c4](https://github.com/special-place-ai-heaven/coding-supervisor/commit/130d5c45dac847d17e9b7859ba19cb0ed4eafe7b))
* say a person runs the task commands the gate refuses from agents ([c2c9626](https://github.com/special-place-ai-heaven/coding-supervisor/commit/c2c96262f58b0aee216a40fc90128fb473c83076))

## [0.19.2](https://github.com/special-place-ai-heaven/coding-supervisor/compare/v0.19.1...v0.19.2) (2026-11-11)


### Bug Fixes

* **claude-code:** judge a dotdot through a missing directory where it lands ([17eedb2](https://github.com/special-place-ai-heaven/coding-supervisor/commit/17eedb2f10495f472eb5c83775fc4d740033cd9a))
* **claude-code:** refuse a dotdot the path check cannot fold ([0264fb6](https://github.com/special-place-ai-heaven/coding-supervisor/commit/0264fb6d71c11469903429a59d15f4decec65e65))
* decide the stop wait on every gap before the refusal list is shortened ([306f838](https://github.com/special-place-ai-heaven/coding-supervisor/commit/306f838d2b755053f610856268c0322148b38e41))
* earlier and fuller gate refusals, MCP refusal at a task limit, dot-dot paths judged where they land ([feb0ce0](https://github.com/special-place-ai-heaven/coding-supervisor/commit/feb0ce0aaf34e5d05bd529c85fac6dfeff9e6c15))
* **mcp:** refuse accept_try and task_complete at a reached task limit ([1851196](https://github.com/special-place-ai-heaven/coding-supervisor/commit/185119630c0b5c624d24c08a5d7fb01f22df641a))
* name an unapproved run environment before the run and every unmet check at Stop ([b56c8e3](https://github.com/special-place-ai-heaven/coding-supervisor/commit/b56c8e3f11965c1d4e5f618cabf8ff282c307a20))
* name every check never run after a missing or stale receipt refusal ([557021d](https://github.com/special-place-ai-heaven/coding-supervisor/commit/557021d062d65a02a15efbc2c59c51db125cd4c2))
* refuse an unapproved environment early only for a journaled root run ([ef61a42](https://github.com/special-place-ai-heaven/coding-supervisor/commit/ef61a42486e424ea4bb8b0cd4c1f2fd13764a64d))
* verdict and status report runs a stop has not counted yet ([84e573a](https://github.com/special-place-ai-heaven/coding-supervisor/commit/84e573a8766538a9f65297e88d879ef48f9f776a))

## [0.19.1](https://github.com/special-place-ai-heaven/coding-supervisor/compare/v0.19.0...v0.19.1) (2026-11-11)


### Bug Fixes

* a pull request whose head is already on the default branch needs no receipt ([a9cf093](https://github.com/special-place-ai-heaven/coding-supervisor/commit/a9cf093d2cbd669bbdea6564a6ab772419b390fe))
* receipt export refuses changed paths outside the contract's scope ([e516e38](https://github.com/special-place-ai-heaven/coding-supervisor/commit/e516e38e79024e7380eb077311a6621f2660d781))
* receipt follow-ups (merged head, export scope check, test time limits) ([d43effc](https://github.com/special-place-ai-heaven/coding-supervisor/commit/d43effc2ed1c1c96e643cf5e9a97c7b9877dd809))
* receipt verify accepts a pull request that changes documentation beside code ([69b3cc9](https://github.com/special-place-ai-heaven/coding-supervisor/commit/69b3cc916040307138f2c502c0df25e96f5c6b50))
* review follow-ups for the receipt exemption, export refusal and nextest install ([02a9f74](https://github.com/special-place-ai-heaven/coding-supervisor/commit/02a9f748f5a5f33a589cb92b3e5bad7fd3597f1f))

## [0.19.0](https://github.com/special-place-ai-heaven/coding-supervisor/compare/v0.18.2...v0.19.0) (2026-11-11)


### Features

* **receipt:** export, verify and require receipt bundles ([442c988](https://github.com/special-place-ai-heaven/coding-supervisor/commit/442c9885d8ab6eaf6322f909b20aef304a1b3e76))
* **receipt:** receipts exported with the code and verified in CI (WP7) ([7f9ae7b](https://github.com/special-place-ai-heaven/coding-supervisor/commit/7f9ae7b5d9e1ef882e24045fa72d2caf6ad561e6))


### Bug Fixes

* **receipt:** check the contract's base_sha always; name a git timeout ([43c3e46](https://github.com/special-place-ai-heaven/coding-supervisor/commit/43c3e4638bbfdd1e34a459fb4f004ea21469cebb))
* **receipt:** grant the receipt job's reads to release.yml's CI call; pin export order ([dd855ee](https://github.com/special-place-ai-heaven/coding-supervisor/commit/dd855ee377e5dd3ed1a34a5f6098b128a61dd123))
* **receipt:** harden bundle export and verify ([dba19c2](https://github.com/special-place-ai-heaven/coding-supervisor/commit/dba19c2d2b95308b1aac2200a72ec9b9ac8a3cef))
* **receipt:** tighten the receipt exemption rule ([70af0f2](https://github.com/special-place-ai-heaven/coding-supervisor/commit/70af0f24c208c68de1df109b19f8891c64faae45))

## [0.18.2](https://github.com/special-place-ai-heaven/coding-supervisor/compare/v0.18.1...v0.18.2) (2026-11-11)


### Bug Fixes

* **gate:** a task is not charged for files already changed at its start (G48) ([b1f861b](https://github.com/special-place-ai-heaven/coding-supervisor/commit/b1f861b7de2a7e62bfbbe9f350fa47a6d09299a2))
* **gate:** a task is not charged for files already changed at its start (G48) ([6f834a0](https://github.com/special-place-ai-heaven/coding-supervisor/commit/6f834a0b5b3c5cbcd4846a79c1b501994038d02c))

## [0.18.1](https://github.com/special-place-ai-heaven/coding-supervisor/compare/v0.18.0...v0.18.1) (2026-11-10)


### Bug Fixes

* **gate:** a prompt after completion advises a new task (G47) ([fe87ff2](https://github.com/special-place-ai-heaven/coding-supervisor/commit/fe87ff246cd4bc954b9c445eff533c9591e7e005))
* **gate:** a prompt after completion advises a new task (G47) ([0fa85f4](https://github.com/special-place-ai-heaven/coding-supervisor/commit/0fa85f4cb82ab69647b5458bd185a14119cc05a8))

## [0.18.0](https://github.com/special-place-ai-heaven/coding-supervisor/compare/v0.17.3...v0.18.0) (2026-11-10)


### Features

* **receipt:** signed receipts and a dependency advisory check (WP6) ([d3053eb](https://github.com/special-place-ai-heaven/coding-supervisor/commit/d3053eb0099ccaad0e0ab22d664c03e9382ff4d2))

## [0.17.3](https://github.com/special-place-ai-heaven/coding-supervisor/compare/v0.17.2...v0.17.3) (2026-11-10)


### Bug Fixes

* **gate:** checks scale to what changed (G46) ([1f41bac](https://github.com/special-place-ai-heaven/coding-supervisor/commit/1f41bacedc9e0e500cd2598543d6e15dfac1ae2e))

## [0.17.2](https://github.com/special-place-ai-heaven/coding-supervisor/compare/v0.17.1...v0.17.2) (2026-11-10)


### Bug Fixes

* **gate:** a session can ask for help before an intent and in discussion (G45) ([0cbac16](https://github.com/special-place-ai-heaven/coding-supervisor/commit/0cbac1615ffc59a06558be86e9198d40c22aa2fa))

## [0.17.1](https://github.com/special-place-ai-heaven/coding-supervisor/compare/v0.17.0...v0.17.1) (2026-11-10)


### Bug Fixes

* **gate:** a subagent that left the tree unchanged hands back its report (G44) ([d5a1b2b](https://github.com/special-place-ai-heaven/coding-supervisor/commit/d5a1b2b4af2e82423b20e0c07cd33273c35858bb))

## [0.17.0](https://github.com/special-place-ai-heaven/coding-supervisor/compare/v0.16.0...v0.17.0) (2026-11-10)


### Features

* **gate:** reviewer record (WP5) ([f435f38](https://github.com/special-place-ai-heaven/coding-supervisor/commit/f435f380899a5ab41cd38216c617b67d7867e48a))
* **gate:** reviewer record (WP5) ([7f32a47](https://github.com/special-place-ai-heaven/coding-supervisor/commit/7f32a47b0b35563e400b9198da1e861f017674e5))


### Bug Fixes

* **gate:** a contract revision drops approvals and keeps rejections ([236e162](https://github.com/special-place-ai-heaven/coding-supervisor/commit/236e162fbbfa3f80740dc611882cb8f39bc70a05))
* **gate:** a new session takes over a task whose session ended (G43) ([f0f41aa](https://github.com/special-place-ai-heaven/coding-supervisor/commit/f0f41aa7aae4bf9f1db8720fa5f8809d45735c59))
* **gate:** a review refusal names task cancel as the way out ([26d3187](https://github.com/special-place-ai-heaven/coding-supervisor/commit/26d3187be1408ca4ca687ee5e879e8e81a3d7b22))
* **gate:** an approval binds to the content the reviewer was launched on ([dc4c7ff](https://github.com/special-place-ai-heaven/coding-supervisor/commit/dc4c7ff0b3f4300f71fb86b2d0a5e20042a7a1fe))
* **gate:** an unreadable launch record counts as unknown content ([358078e](https://github.com/special-place-ai-heaven/coding-supervisor/commit/358078ee6a7faa1b864628234120c76a02a6950a))
* **gate:** append the mutation row under the state lock ([3d1ab71](https://github.com/special-place-ai-heaven/coding-supervisor/commit/3d1ab714e6f3f28dfc18e0fd76e32b61725b239b))
* **gate:** read a rejection loosely, an approval exactly ([42ddf71](https://github.com/special-place-ai-heaven/coding-supervisor/commit/42ddf71bf6cffcf9b497b98f662c652258c166ae))
* **gate:** read mutations from the transaction-written journal too ([bd16cf2](https://github.com/special-place-ai-heaven/coding-supervisor/commit/bd16cf2c1de22494a5165d3a73357b0d1f73d610))
* **gate:** record a rejection on a pending or discussion turn ([bc01de8](https://github.com/special-place-ai-heaven/coding-supervisor/commit/bc01de8a1bc68f4fa0a492b795c6eb38f36c278a))
* **gate:** record a rejection on a refused pending or changed-intent turn ([d87a46b](https://github.com/special-place-ai-heaven/coding-supervisor/commit/d87a46b57b86ffda082c3bfe5b9aa6f8309929bf))
* **gate:** review summary says whether the rule is met ([5ee9e80](https://github.com/special-place-ai-heaven/coding-supervisor/commit/5ee9e809b44b05e0641f17d8550bd67bc99fe931))
* **gate:** reviews follow the full content, documentation included ([3e411a2](https://github.com/special-place-ai-heaven/coding-supervisor/commit/3e411a20e7e822822831eca3efac49017628a128))
* **gate:** strip control characters from review findings ([b02368e](https://github.com/special-place-ai-heaven/coding-supervisor/commit/b02368edaed80b86061e88e7cfae9cd446b558e7))
* **gate:** the limit report names an unmet review ([5a0ed79](https://github.com/special-place-ai-heaven/coding-supervisor/commit/5a0ed79822de4b15dbff905d031e813625dcbee0))
* **gate:** the review refusal names the read-only set the gate uses ([e5b4f27](https://github.com/special-place-ai-heaven/coding-supervisor/commit/e5b4f27b852b335402aee5f8201ef6a239357996))
* **gate:** unreadable launch content drops a background approval ([30a76b3](https://github.com/special-place-ai-heaven/coding-supervisor/commit/30a76b3488731a6a1e3ed4274eb1c76d5afcba99))
* **gate:** word the mutation journal refusal like other fail-closed texts ([d02a3b2](https://github.com/special-place-ai-heaven/coding-supervisor/commit/d02a3b282c7af400e0847ca1c2bc20de8dc79650))
* **gate:** write the mutation row inside the hook transaction ([317dcf8](https://github.com/special-place-ai-heaven/coding-supervisor/commit/317dcf87744a5fb8ed6062b4ce72083db81fe5ce))


### Performance Improvements

* **gate:** build the content tree only when a review can matter ([02b20f0](https://github.com/special-place-ai-heaven/coding-supervisor/commit/02b20f0848ae44b00bdbb1e62e23f41bd5dbd101))

## [0.16.0](https://github.com/special-place-ai-heaven/coding-supervisor/compare/v0.15.0...v0.16.0) (2026-11-10)


### Features

* **gate:** enforce task time and turn limits ([6719066](https://github.com/special-place-ai-heaven/coding-supervisor/commit/67190666b2db1a666929c3d3e37336bb6095d274))
* **gate:** enforce task time and turn limits ([0f103a1](https://github.com/special-place-ai-heaven/coding-supervisor/commit/0f103a1687779552e5f792325d0e46dafa1d6650))


### Bug Fixes

* **gate:** task limits review round 1 ([e874ace](https://github.com/special-place-ai-heaven/coding-supervisor/commit/e874ace6570fa40593facf6406b53a4d1d6e6fe3))
* **gate:** task limits review round 2 ([94e4510](https://github.com/special-place-ai-heaven/coding-supervisor/commit/94e451004d06b4ef23a3dab44bdb12a3319076ec))
* **gate:** task limits review round 3 ([5326954](https://github.com/special-place-ai-heaven/coding-supervisor/commit/532695466bed00d04252e9138097a336adbd1874))

## [0.15.0](https://github.com/special-place-ai-heaven/coding-supervisor/compare/v0.14.0...v0.15.0) (2026-11-10)


### Features

* **gate:** canonical scope paths refuse unresolvable links ([e235347](https://github.com/special-place-ai-heaven/coding-supervisor/commit/e23534706058a98faee116b80ebb23c6db62b6db))
* **gate:** contract v2 with explicit v1 migration ([96ed255](https://github.com/special-place-ai-heaven/coding-supervisor/commit/96ed2553e1aa3df72b7b1d32dc70b1ed4798a86d))
* **gate:** task contract v2 with explicit v1 migration ([ef0211e](https://github.com/special-place-ai-heaven/coding-supervisor/commit/ef0211eec64bd9771c3981341152f8574a0f4fef))


### Bug Fixes

* **gate:** check the saved contract and keep v1 loading as on main ([073c2af](https://github.com/special-place-ai-heaven/coding-supervisor/commit/073c2af88968c4f609c73f994d1bb29c3bc8df6f))
* **gate:** refuse a caller's newer schema before stamping at start ([942422f](https://github.com/special-place-ai-heaven/coding-supervisor/commit/942422f54501fd46e84df6de25e85bf94212ce65))

## [0.14.0](https://github.com/special-place-ai-heaven/coding-supervisor/compare/v0.13.4...v0.14.0) (2026-11-10)


### Features

* **gate:** fail closed on adapter errors for governed projects ([95e7133](https://github.com/special-place-ai-heaven/coding-supervisor/commit/95e713318a16a1f71f76fd0dade2b563241deb48))


### Bug Fixes

* **cli:** write the failure mode first and show it on every level write ([922f974](https://github.com/special-place-ai-heaven/coding-supervisor/commit/922f97497287d32a3475f72ee660e30800e2a4a9))
* **gate:** route every adapter error through one fail-closed path ([fbbe473](https://github.com/special-place-ai-heaven/coding-supervisor/commit/fbbe4738a48803fb1db153208ba25d833dfc4217))
* **gate:** route the paused-state read error through adapter_error ([fda1956](https://github.com/special-place-ai-heaven/coding-supervisor/commit/fda19563de9772274e0ba5e541c685d37aedae44))

## [0.13.4](https://github.com/special-place-ai-heaven/coding-supervisor/compare/v0.13.3...v0.13.4) (2026-11-09)


### Bug Fixes

* **gate:** colliding subfolder names keep distinct check ids (G41) ([35606b5](https://github.com/special-place-ai-heaven/coding-supervisor/commit/35606b5a58f9b6f265f6a0f35ef5dfccf043c9a7))
* **gate:** read-only forge, worktree and registry views stay open (G40) ([f61038b](https://github.com/special-place-ai-heaven/coding-supervisor/commit/f61038b53df0bc489eee99d1ee0496bf16c1cf29))

## [0.13.3](https://github.com/special-place-ai-heaven/coding-supervisor/compare/v0.13.2...v0.13.3) (2026-11-09)


### Bug Fixes

* **gate:** read one documentation list, .github/docs-paths.txt, in the gate and CI ([b878be5](https://github.com/special-place-ai-heaven/coding-supervisor/commit/b878be57b32ed6a11c43c7e49da24a6f34f5a31a))
* **gate:** read one documentation list, .github/docs-paths.txt, in the gate and CI ([a4805b1](https://github.com/special-place-ai-heaven/coding-supervisor/commit/a4805b1fbdcbe622b5d93087e4f7e6767be962d3))

## [0.13.2](https://github.com/special-place-ai-heaven/coding-supervisor/compare/v0.13.1...v0.13.2) (2026-11-09)


### Bug Fixes

* **gate:** keep the index time on the scratch copy so a same-second edit counts ([3517975](https://github.com/special-place-ai-heaven/coding-supervisor/commit/3517975c6af618ea41c9c736a41e0649c83812a4))
* **gate:** keep the index time on the scratch copy so a same-second edit counts ([634bc39](https://github.com/special-place-ai-heaven/coding-supervisor/commit/634bc39482df41e255edc366d70ea9ffe67f7f70))

## [0.13.1](https://github.com/special-place-ai-heaven/coding-supervisor/compare/v0.13.0...v0.13.1) (2026-11-07)


### Bug Fixes

* **gate:** a version query is read-only under any prompt (G38) ([efd1f31](https://github.com/special-place-ai-heaven/coding-supervisor/commit/efd1f3136c10e4aaeb3650e4e58a4472250668b2))
* **gate:** G39 review follow-ups (revalidate from cwd, ceiling comment) ([e6c848c](https://github.com/special-place-ai-heaven/coding-supervisor/commit/e6c848c7d61ddce21d3ffccedc55458a103ea244))
* **gate:** no git under the state lock when a prompt or proposal runs (G26) ([e777512](https://github.com/special-place-ai-heaven/coding-supervisor/commit/e777512f10cd9da0aea5d33351294a459afa8fe3))
* **gate:** read-only version queries, no git under the lock, cached non-repos (G38, G26, G39) ([facb97f](https://github.com/special-place-ai-heaven/coding-supervisor/commit/facb97f3b22e3d00f549bae2aed32417c513e742))
* **gate:** remember "not a repository" so replay does not respawn git (G39) ([e5a0a48](https://github.com/special-place-ai-heaven/coding-supervisor/commit/e5a0a482559e058fb69c4b6f77ab321d9cd06364))
* **gate:** review follow-ups for G38 and G26 ([83e35d9](https://github.com/special-place-ai-heaven/coding-supervisor/commit/83e35d9017b9c3a0924c8fbf5035cf344c2e3943))

## [0.13.0](https://github.com/special-place-ai-heaven/coding-supervisor/compare/v0.12.0...v0.13.0) (2026-11-07)


### Features

* **gate:** person-declared checks and subfolder Cargo discovery (G32, G33) ([9f60e12](https://github.com/special-place-ai-heaven/coding-supervisor/commit/9f60e128c7abfc48b946cb45bfe4f539f1371314))


### Bug Fixes

* **gate:** AAP report fixes G32-G37 ([c1e919e](https://github.com/special-place-ai-heaven/coding-supervisor/commit/c1e919e4c7f7136603d9afe34e7b6f69f9758b40))
* **gate:** full contract schema, checkless prompt line, notices (G34-G37) ([ca96557](https://github.com/special-place-ai-heaven/coding-supervisor/commit/ca965578637174f70dcc77101e364802f1d9298e))
* **gate:** keep the discussion call shape in the checkless line (F19) ([5909105](https://github.com/special-place-ai-heaven/coding-supervisor/commit/59091050f1223c481387da82c260f00027fd1c17))
* **gate:** review follow-ups for add-check (G32) ([574571f](https://github.com/special-place-ai-heaven/coding-supervisor/commit/574571fc970385905751446c9f6e74d18c72003f))

## [0.12.0](https://github.com/special-place-ai-heaven/coding-supervisor/compare/v0.11.1...v0.12.0) (2026-11-06)


### Features

* **gate:** edits to docs-only paths keep the evidence (G28) ([cec92f6](https://github.com/special-place-ai-heaven/coding-supervisor/commit/cec92f6ac3030b472c485eb8d8b675cc431adf6f))
* **gate:** warn when the installed package is newer than the running binary (G23) ([4292768](https://github.com/special-place-ai-heaven/coding-supervisor/commit/429276896a0bab32e4c3f4f706fe5cae1595ae05))


### Bug Fixes

* **gate:** allow memory saves under discussion and unclassified prompts (G31) ([c783dfb](https://github.com/special-place-ai-heaven/coding-supervisor/commit/c783dfbca7f5b7f531970b56ecf35735b5b44c4b))
* **gate:** let the chair record a worker's TC job exit (G25) ([a074004](https://github.com/special-place-ai-heaven/coding-supervisor/commit/a074004401d4d0033734af1080f1571961a9af06))
* **gate:** refuse a mutation the gate cannot journal (G24b) ([d9c3fed](https://github.com/special-place-ai-heaven/coding-supervisor/commit/d9c3fed108940437db9cbf1cd7ae02d3e120c6b7))
* **gate:** refuse a still-running TC check once per turn, then wait (G27) ([3b7c1b8](https://github.com/special-place-ai-heaven/coding-supervisor/commit/3b7c1b8bcd3f58730548dc42c02c2f428b22bcb0))
* **gate:** review fixes for G28, G30 and G31 ([b49f9ec](https://github.com/special-place-ai-heaven/coding-supervisor/commit/b49f9ec1b479e5a8a2079d9593d68ba787331b73))
* **gate:** usability rows G23-G31 ([ae93250](https://github.com/special-place-ai-heaven/coding-supervisor/commit/ae932507898e1966f0bded58158f5053f895ed56))
* **mcp:** task_state shows criteria Stop will count from observed runs (G30) ([9f6b62d](https://github.com/special-place-ai-heaven/coding-supervisor/commit/9f6b62dca3c6db84806fab7608bb67e4cc4f2cfa))

## [0.11.1](https://github.com/special-place-ai-heaven/coding-supervisor/compare/v0.11.0...v0.11.1) (2026-11-05)


### Bug Fixes

* **gate:** remove the dead ends and noise found dogfooding (G15-G22) ([0d23c6d](https://github.com/special-place-ai-heaven/coding-supervisor/commit/0d23c6d0aa8f5de712d773f3dfbce812a5170a5a))
* **gate:** remove the dead ends and noise found dogfooding (G15-G22) ([d689b53](https://github.com/special-place-ai-heaven/coding-supervisor/commit/d689b53cb9470ea2d988263e06251422184132b4))

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
