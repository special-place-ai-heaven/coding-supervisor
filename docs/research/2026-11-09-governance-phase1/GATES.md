# Gates: governance Phase 1 (local only)

OWNS: .github/workflows/**, src/**, tests/**, adapters/**, cli/**, docs/**, CLAUDE.md, HANDOVER.md, supervisor-keys.json, supervisor-receipts/**

Scope: every work package in PLAN.md is merged into main or abandoned here with a reason, and main passes the supervisor gate's own commands.

How to run: from the main checkout at `E:\project\coding-supervisor` after `git pull`, with `CARGO_TARGET_DIR=C:\Target\coding-supervisor\main\target`, through Terminal Commander:
`node C:\Users\rakovnik\.claude\skills\unlazy\scripts\gate-check.mjs --timeout 3600 --root . --cwd . docs/research/2026-11-09-governance-phase1/GATES.md`.
The cargo gates use the commands the supervisor contract (`supervisor-contract.json` C3) and `ci.yml` check. Where a command prints nothing on success, `&& echo <marker>` is appended and the command itself is unchanged. The `merged` oracle also fails unless the checkout is at `origin/main`, so every later gate measures the merged code. The `--test <file>` names are part of this contract: each package must use exactly those test-file names. `docs-only-ci` has no passing control before WP0 lands. Its control is PR D itself. W7.3 passed by the owner's `receipt-exempt` label on PR #96, not by a receipt (no governed run completed then); W7.8 is the gate for a real receipt. `ci-failed` reads the branch's own workflow runs, because the checks list of a pull request shows the newest result for its head commit, which another pull request on the same commit can replace (PR #97 shares its head with PR #96).

- [x] G0: this ledger states outcomes that can fail
  CHECK: node C:\Users\rakovnik\.claude\skills\unlazy\scripts\gate-lint.mjs docs/research/2026-11-09-governance-phase1/GATES.md
  EXPECT: LINT OK
  EVIDENCE: automatic-evidence=v1; definition-sha256=9af8337630cfc772d47f23b4236bdc9cfc08ac1ba1ce41cc4be394d1ff88c9ff; exit=0; EXPECT=matched; output-sha256=b6036c16014bf6f430df81b3eb49b1f70fe567cc6e178e365bccc514634b3894; output-bytes=585; shell=C:\WINDOWS\system32\cmd.exe; cwd=E:\project\cs-wt\docs-ledger; path=7b67f14e9903/87 entries

- [x] W0.1: the CI change that reports every pull request is merged into main
  CHECK: node docs/research/2026-11-09-governance-phase1/check.mjs merged ci/report-every-pr
  EXPECT: merged into main: ci/report-every-pr
  EVIDENCE: automatic-evidence=v1; definition-sha256=1652f356dbdf6395b511ba2fc3435aa7227bd82dc66c328c30ce11bb206158e0; exit=0; EXPECT=matched; output-sha256=8ebb8e5e8515a23b7165a35ce64f6f3fca8e65f6313a4e3456aba38270931327; output-bytes=41; shell=C:\WINDOWS\system32\cmd.exe; cwd=E:\project\cs-wt\docs-ledger; path=7b67f14e9903/87 entries

- [x] W0.2: a docs-only PR got all three CI checks, the decide job passing and both heavy jobs skipped
  CHECK: node docs/research/2026-11-09-governance-phase1/check.mjs docs-only-ci docs/governance-phase1-plan
  EXPECT: heavy ones skipped
  EVIDENCE: automatic-evidence=v1; definition-sha256=57aef9b317b838f543e4723ac4e758ad951523670bf0c96e93ba49d5100cba9c; exit=0; EXPECT=matched; output-sha256=d0fb5dd57dd610adaf05b9f5c8a656dcea5249f9d59ef90ebea4268abaf8455c; output-bytes=58; shell=C:\WINDOWS\system32\cmd.exe; cwd=E:\project\cs-wt\docs-ledger; path=7b67f14e9903/87 entries

- [x] W0.3: the owner was told the exact check names to require in ruleset 26854669
  EVIDENCE: told in session on 2026-11-11 (`Decide whether to run`, `Linux: format, lint, test, core dependency check`, `Windows (hosted): test`, and later `Verify supervisor receipts`). The owner then decided against branch protection: "no need for branch protection since any commiter needs to be vetted by me", with the standing rule that no agent merges while a check is red.

- [x] W1.1: fail-closed by default is merged into main
  CHECK: node docs/research/2026-11-09-governance-phase1/check.mjs merged feat/fail-closed
  EXPECT: merged into main: feat/fail-closed
  EVIDENCE: automatic-evidence=v1; definition-sha256=e4162d56443755534116231fd5673858995264cbe126d6217bed83ed34d07b9d; exit=0; EXPECT=matched; output-sha256=68af76469866064e5b7eaa0b78d2ce27983ee56d422a3556f682873b717f4839; output-bytes=39; shell=C:\WINDOWS\system32\cmd.exe; cwd=E:\project\cs-wt\docs-ledger; path=7b67f14e9903/87 entries

- [x] W1.2: an adapter error blocks a governed task and the explicit fail-open setting is logged as advisory
  CHECK: cargo test -p supervisor-claude-code --test fail_closed
  EXPECT: /test result: ok\. [1-9][0-9]* passed/
  EVIDENCE: automatic-evidence=v1; definition-sha256=04cb3227437c3d4c81f6cc5dffb03f58f3a05cbf730eaee8277a22e9e30c07fc; exit=0; EXPECT=matched; output-sha256=00cc7f8708ca0db1c1bb361b759c55d77a1cdc2fe922f14e88de3abb38f76399; output-bytes=1597; shell=C:\WINDOWS\system32\cmd.exe; cwd=E:\project\cs-wt\docs-ledger; path=7b67f14e9903/87 entries

- [x] W1.3: while the hooks fail closed, turning the gate off from a plain terminal succeeds and the next hook call no longer blocks (spec section 9 case 18)
  CHECK: cargo test -p supervisor --test fail_closed_escape
  EXPECT: /test result: ok\. [1-9][0-9]* passed/
  EVIDENCE: automatic-evidence=v1; definition-sha256=d095c542f194ebbe333da28e1b0215645369a87c2fcb779e9cf935f7197355c6; exit=0; EXPECT=matched; output-sha256=a3bc40c4639a835a09a5f2d71308c3d51c80ee18c304a6ec211cb29da93db68a; output-bytes=818; shell=C:\WINDOWS\system32\cmd.exe; cwd=E:\project\cs-wt\docs-ledger; path=7b67f14e9903/87 entries

- [x] W2.1: contract v2 is merged into main
  CHECK: node docs/research/2026-11-09-governance-phase1/check.mjs merged feat/contract-v2
  EXPECT: merged into main: feat/contract-v2
  EVIDENCE: automatic-evidence=v1; definition-sha256=153a51e46371d3db94098e7c18427045d2d2676c7eb5c0ed2abbdc9f1fd9fd38; exit=0; EXPECT=matched; output-sha256=80493c7bbac176001227bc84c9ba186dc24ed02ef34d06043b023bd1bbe75d06; output-bytes=39; shell=C:\WINDOWS\system32\cmd.exe; cwd=E:\project\cs-wt\docs-ledger; path=7b67f14e9903/87 entries

- [x] W2.2: v2 contracts load, v1 contracts load only through the migration, and path escapes are rejected
  CHECK: cargo test -p supervisor-claude-code --test contract_v2
  EXPECT: /test result: ok\. [1-9][0-9]* passed/
  EVIDENCE: automatic-evidence=v1; definition-sha256=83c9f8776981d0327287d8dd7d9674d499d05054c9034ec041ac5bc77140160e; exit=0; EXPECT=matched; output-sha256=26b4310c84cc69852f7fadb44d15e30c23251a83d303c6ea9f745f2048a284f6; output-bytes=1402; shell=C:\WINDOWS\system32\cmd.exe; cwd=E:\project\cs-wt\docs-ledger; path=7b67f14e9903/87 entries

- [x] W2.3: the core comment no longer claims forbidden_effects is unenforced
  CHECK: node docs/research/2026-11-09-governance-phase1/check.mjs stale-comment-fixed
  EXPECT: no longer claims it is unenforced
  EVIDENCE: automatic-evidence=v1; definition-sha256=5661caefac57fa23de17e392768995887def6c27c8bfc7dc8f96cbd3955388be; exit=0; EXPECT=matched; output-sha256=ece7cd865cca63723781035ad8c8a4600bd0ddb25d0de3c8897ffa30491fa957; output-bytes=60; shell=C:\WINDOWS\system32\cmd.exe; cwd=E:\project\cs-wt\docs-ledger; path=7b67f14e9903/87 entries

- [x] W4.1: time and turn limits are merged into main
  CHECK: node docs/research/2026-11-09-governance-phase1/check.mjs merged feat/task-limits
  EXPECT: merged into main: feat/task-limits
  EVIDENCE: automatic-evidence=v1; definition-sha256=88225061544b2d69d05dad95c5119c3ce1461131ad810610bcd89f76435c8ca1; exit=0; EXPECT=matched; output-sha256=1b11b429a59304b90f481d1b0d9e2bedcc0b55b1373f738659ebade2d8494a23; output-bytes=39; shell=C:\WINDOWS\system32\cmd.exe; cwd=E:\project\cs-wt\docs-ledger; path=7b67f14e9903/87 entries

- [x] W4.2: reaching a limit denies new work and leaves the diff and journal unchanged
  CHECK: cargo test -p supervisor-claude-code --test limits
  EXPECT: /test result: ok\. [1-9][0-9]* passed/
  EVIDENCE: automatic-evidence=v1; definition-sha256=724431d1907e4f8cc8f5c5f4eb37eacfa55a409ca9b321e6d9f28d383b0e1ce5; exit=0; EXPECT=matched; output-sha256=a04404ec1f08630b90b82bc4e3bd848051afd292a8a0832ba6704bc7a0e75259; output-bytes=2579; shell=C:\WINDOWS\system32\cmd.exe; cwd=E:\project\cs-wt\docs-ledger; path=7b67f14e9903/87 entries

- [x] W5.1: the reviewer record is merged into main
  CHECK: node docs/research/2026-11-09-governance-phase1/check.mjs merged feat/review-record
  EXPECT: merged into main: feat/review-record
  EVIDENCE: automatic-evidence=v1; definition-sha256=60b5d7289e9052894ccee03b97fa00fe952190cfbd2a705e7ed9ee4dacea7917; exit=0; EXPECT=matched; output-sha256=6789cfa150273483ac2059a019c0edba723305e1403e30ee9298010e23fcff01; output-bytes=41; shell=C:\WINDOWS\system32\cmd.exe; cwd=E:\project\cs-wt\docs-ledger; path=7b67f14e9903/87 entries

- [x] W5.2: a builder-identity review is stored caller-asserted and does not satisfy an independent-review contract
  CHECK: cargo test -p supervisor-claude-code --test review_record
  EXPECT: /test result: ok\. [1-9][0-9]* passed/
  EVIDENCE: automatic-evidence=v1; definition-sha256=666b731250535af602c6614cba84e83a2830eda9ad344e8ab7716d23d7139f6d; exit=0; EXPECT=matched; output-sha256=d6b5de174d1c059f65d2c463694cdb521620e51cfaee5b82dc81f304713f29d5; output-bytes=2721; shell=C:\WINDOWS\system32\cmd.exe; cwd=E:\project\cs-wt\docs-ledger; path=7b67f14e9903/87 entries

- [x] W6.1: signed receipts are merged into main
  CHECK: node docs/research/2026-11-09-governance-phase1/check.mjs merged feat/signed-receipts
  EXPECT: merged into main: feat/signed-receipts
  EVIDENCE: automatic-evidence=v1; definition-sha256=4960a11f34efa1e01bcf2833051f518e111e79fd554ec84daf61b64e99f3107f; exit=0; EXPECT=matched; output-sha256=5d706fbf787286f170ac7362d2e3fdc60adb3f5335dc62eca7a5fbeb3f141a26; output-bytes=43; shell=C:\WINDOWS\system32\cmd.exe; cwd=E:\project\cs-wt\docs-ledger; path=7b67f14e9903/87 entries

- [x] W6.2: a receipt signs and verifies, a changed byte or revoked key fails, and a missing key yields unsigned
  CHECK: cargo test -p supervisor --test receipt_sign
  EXPECT: /test result: ok\. [1-9][0-9]* passed/
  EVIDENCE: automatic-evidence=v1; definition-sha256=8fbf7a9aaedd5d1e1f98417416092f3c9078bb3c572d2365b4e30b79724dbaab; exit=0; EXPECT=matched; output-sha256=6ddb7513724fb8081daacac3863085b84109bfd3370db41551e7b7d08c71e8d5; output-bytes=1471; shell=C:\WINDOWS\system32\cmd.exe; cwd=E:\project\cs-wt\docs-ledger; path=7b67f14e9903/87 entries

- [x] W7.1: receipt export, verify and the CI job are merged into main
  CHECK: node docs/research/2026-11-09-governance-phase1/check.mjs merged feat/receipt-verify-ci
  EXPECT: merged into main: feat/receipt-verify-ci
  EVIDENCE: automatic-evidence=v1; definition-sha256=244df40a335e4f773bb963a3ebf73342434a536e32e3fe20a41db8091606e371; exit=0; EXPECT=matched; output-sha256=d29c61ab7fbb16e8abd60633d203f82c6cb1d3d06ddbe98e96ceba51565e35f0; output-bytes=45; shell=C:\WINDOWS\system32\cmd.exe; cwd=E:\project\cs-wt\docs-ledger; path=7b67f14e9903/87 entries

- [x] W7.2: verify rejects a wrong signature, contract hash, out-of-scope path and stale tree
  CHECK: cargo test -p supervisor --test receipt_verify
  EXPECT: /test result: ok\. [1-9][0-9]* passed/
  EVIDENCE: automatic-evidence=v1; definition-sha256=08525da7352e7307d446da77e143ee4a7a43328efd59964869c4c31b3b1b4177; exit=0; EXPECT=matched; output-sha256=5d68a6429146afd28a02f203b28125f687a65fcad71c58932d0e1a0948a11fc4; output-bytes=1617; shell=C:\WINDOWS\system32\cmd.exe; cwd=E:\project\cs-wt\docs-ledger; path=7b67f14e9903/87 entries

- [x] W7.3: the receipt verification job ran and passed in CI on its own PR
  CHECK: node docs/research/2026-11-09-governance-phase1/check.mjs ci-passed feat/receipt-verify-ci "Verify supervisor receipts"
  EXPECT: job passed on PR
  EVIDENCE: automatic-evidence=v1; definition-sha256=6e8b78d552900bba88951a178f58f24543ff3b9b85772620cb569ac1289d4dbb; exit=0; EXPECT=matched; output-sha256=7104bb9600f7f883cf52dd9dce83afcad499c134862cbb31ac8a359763db68a9; output-bytes=49; shell=C:\WINDOWS\system32\cmd.exe; cwd=E:\project\cs-wt\docs-ledger; path=7b67f14e9903/87 entries

- [x] W7.4: committing a receipt into supervisor-receipts/ leaves the candidate identity unchanged, so the receipt still verifies against the commit that contains it
  CHECK: cargo test -p supervisor --test receipt_in_tree
  EXPECT: /test result: ok\. [1-9][0-9]* passed/
  EVIDENCE: automatic-evidence=v1; definition-sha256=3c99266d2e6fa0a68bdfb075155ef1954aac7cf77de4b0a2ddea44269b6d7a35; exit=0; EXPECT=matched; output-sha256=baa14c786a4481a584a7b0118ea2c72214ef6640385844fd9641e72e49127b1e; output-bytes=673; shell=C:\WINDOWS\system32\cmd.exe; cwd=E:\project\cs-wt\docs-ledger; path=7b67f14e9903/87 entries

- [x] W7.5: a PR to main with no receipt and no exemption fails the receipt job in CI
  CHECK: node docs/research/2026-11-09-governance-phase1/check.mjs ci-failed test/receipt-required "Verify supervisor receipts"
  EXPECT: job failed on PR
  EVIDENCE: automatic-evidence=v1; definition-sha256=2a86fd9e8aa630f91de04b97ae4c91c743482021acd70f8151c21fd409bc11d2; exit=0; EXPECT=matched; output-sha256=9fd663693ff20517573056ad57c504f1927a40556b68627b179ba6470a085f64; output-bytes=88; shell=C:\WINDOWS\system32\cmd.exe; cwd=E:\project\cs-wt\docs-ledger; path=7b67f14e9903/87 entries

- [x] W7.6: the exemption rule is decided in CI only by docs-only, release-please or an owner-applied label, never by branch name
  CHECK: cargo test -p supervisor --test receipt_exemption
  EXPECT: /test result: ok\. [1-9][0-9]* passed/
  EVIDENCE: automatic-evidence=v1; definition-sha256=4c71434fcee754321c163a817aefbd45dae86d9cecf36e334af2ef942943ad40; exit=0; EXPECT=matched; output-sha256=7efd51651974f2044baa46cce811a61c0a53fca7caaf3e34a541e03d77a11056; output-bytes=1244; shell=C:\WINDOWS\system32\cmd.exe; cwd=E:\project\cs-wt\docs-ledger; path=7b67f14e9903/87 entries

- [x] W7.7: the receipt follow-ups (a head already on main needs no receipt, export checks the scope, verify accepts documentation changed beside code) are merged into main
  CHECK: node docs/research/2026-11-09-governance-phase1/check.mjs merged fix/receipt-followups
  EXPECT: merged into main: fix/receipt-followups
  EVIDENCE: automatic-evidence=v1; definition-sha256=065cd20cbef128646463b147b6748a2f9668da8e2b3e92a1dc94654a40fbf207; exit=0; EXPECT=matched; output-sha256=2bf7927a20e6cff229e938adbf48e599a6ba8db8ce549f72b50452d304747a99; output-bytes=44; shell=C:\WINDOWS\system32\cmd.exe; cwd=E:\project\cs-wt\docs-ledger; path=7b67f14e9903/87 entries

- [x] W7.8: a pull request carrying a signed receipt from a live governed run passed the receipt job in CI without an exemption label
  CHECK: node docs/research/2026-11-09-governance-phase1/check.mjs receipt-passed fix/receipt-followups "Verify supervisor receipts"
  EXPECT: job passed with a receipt bundle on PR
  EVIDENCE: automatic-evidence=v1; definition-sha256=0d50d5f5421d6682c51b4ac34783169d009a4592f68c9b893ffeb54da194466e; exit=0; EXPECT=matched; output-sha256=8d887b2fa549a86cb78d6e0c7649e91d5a54eb01700e8e7e4db5f6617a0648a3; output-bytes=71; shell=C:\WINDOWS\system32\cmd.exe; cwd=E:\project\cs-wt\docs-ledger; path=7b67f14e9903/87 entries

- [x] W3.1: worktree creation is merged into main
  CHECK: node docs/research/2026-11-09-governance-phase1/check.mjs merged feat/worktree-create
  EXPECT: merged into main: feat/worktree-create
  EVIDENCE: automatic-evidence=v1; definition-sha256=8fcf3b44dd9418be028895cd5cd2fc2e1d5d195e6b40242e351efdb22c510c52; exit=0; EXPECT=matched; output-sha256=6e95f0585a6beaa3f4617b3ed5f215d67e18bf06443d770e27d987fe4cd28bfb; output-bytes=44; shell=C:\WINDOWS\system32\cmd.exe; cwd=E:\project\cs-wt\final-ledger; path=7b67f14e9903/87 entries

- [x] W3.2: a created worktree is verified for path, branch, base and cleanliness, and cleanup keeps unreceipted work
  CHECK: cargo test -p supervisor --test worktree_create
  EXPECT: /test result: ok\. [1-9][0-9]* passed/
  EVIDENCE: automatic-evidence=v1; definition-sha256=70ebfc9f972c7bedca66daef3519eff60c62f46ce19aeac3244f560b382bf8ee; exit=0; EXPECT=matched; output-sha256=bff4204859f08aae8a0a6391afde55cc58f98606af1acf02264a906b7a1db3cd; output-bytes=2841; shell=C:\WINDOWS\system32\cmd.exe; cwd=E:\project\cs-wt\final-ledger; path=7b67f14e9903/87 entries

- [x] W8.1: the barrier matrix is merged into main
  CHECK: node docs/research/2026-11-09-governance-phase1/check.mjs merged test/barrier-matrix
  EXPECT: merged into main: test/barrier-matrix
  EVIDENCE: automatic-evidence=v1; definition-sha256=21a5ed8db295ad6d34835398b739818e65f4efb3aade225275d2c5a7b49a2959; exit=0; EXPECT=matched; output-sha256=0364b942dbe1346d954fb7787c7e64cfea1d8bf2eecdad02f33c9961bc0e16d0; output-bytes=43; shell=C:\WINDOWS\system32\cmd.exe; cwd=E:\project\cs-wt\final-ledger; path=7b67f14e9903/87 entries

- [x] W8.2: RESULTS.md records all 20 spec section 9 cases with an integer exit code or a server N/A reason
  CHECK: node docs/research/2026-11-09-governance-phase1/check.mjs matrix
  EXPECT: matrix: 20 cases recorded
  EVIDENCE: automatic-evidence=v1; definition-sha256=56cdf55dd067c5124707140cc8efc7e223f236b2ec01140cc69909ed53a35e61; exit=0; EXPECT=matched; output-sha256=3ff5e1f64ceae5e6475843d9d4da18be069e8a8689be7a9926e1c71df8e9b9d5; output-bytes=65; shell=C:\WINDOWS\system32\cmd.exe; cwd=E:\project\cs-wt\final-ledger; path=7b67f14e9903/87 entries

- [x] F1: main passes the formatting check
  CHECK: cargo fmt --all -- --check && echo fmt-clean
  EXPECT: fmt-clean
  EVIDENCE: automatic-evidence=v1; definition-sha256=0c6b92d6801b0ae8ab724f3fd2e4b30e618c59fea8d88b5dc151b4d129da5b95; exit=0; EXPECT=matched; output-sha256=fac742279ceddbed040c22aa709773e18b4342d5670d81c6ac66a8e6ed361bef; output-bytes=11; shell=C:\WINDOWS\system32\cmd.exe; cwd=E:\project\cs-wt\docs-ledger; path=7b67f14e9903/87 entries

- [x] F2: main passes clippy with warnings as errors
  CHECK: cargo clippy --workspace --all-targets -- -D warnings && echo clippy-clean
  EXPECT: clippy-clean
  EVIDENCE: automatic-evidence=v1; definition-sha256=42d1e668c66517af293cdc40f13f6a601e95c8617f6d6566c7919078e3026aaf; exit=0; EXPECT=matched; output-sha256=39d9448758eb9c3b7a02f0b7daef7dd8d72828ee4c6a7a5f71120c65f1e4c68b; output-bytes=400; shell=C:\WINDOWS\system32\cmd.exe; cwd=E:\project\cs-wt\docs-ledger; path=7b67f14e9903/87 entries

- [x] F3: main passes the supervisor regression criterion
  CHECK: cargo test --workspace
  EXPECT: /test result: ok\. [1-9][0-9]* passed/
  EVIDENCE: automatic-evidence=v1; definition-sha256=3db8641fcebe140b0a866f7236f5f7b7a422b19f6dac0f61f113f9bc64bcd666; exit=0; EXPECT=matched; output-sha256=44a9eeafe0203ec6824fc23342970131f45a058afc6f4501b8a789ab1bde7e45; output-bytes=59858; shell=C:\WINDOWS\system32\cmd.exe; cwd=E:\project\cs-wt\docs-ledger; path=7b67f14e9903/87 entries

- [x] F4: the core crate still has no dependencies
  CHECK: node docs/research/2026-11-09-governance-phase1/check.mjs core-no-deps
  EXPECT: core crate has no dependencies
  EVIDENCE: automatic-evidence=v1; definition-sha256=073642a730fb069ffa14d83d4ec10c0cb25ab440c68025f3c4e9112c8921c4e7; exit=0; EXPECT=matched; output-sha256=8269e269d8f4219d0274963a41997bd0dd627d616356c637c70e560b220ab5ac; output-bytes=31; shell=C:\WINDOWS\system32\cmd.exe; cwd=E:\project\cs-wt\docs-ledger; path=7b67f14e9903/87 entries

- [x] F5: CLAUDE.md and HANDOVER.md describe the merged state, with every enforcement claim traced to a named test
  EVIDENCE: done in PR #108 (2026-11-12) and fact-checked by a separate agent with no false statement found. Refusal claims name their test files; the claims no test can pin (results of live harness runs, configured action-policy errors) are marked "(no test names this)" in CLAUDE.md rather than traced.

- [x] F6: every deferral is listed in RESULTS.md with its reason, and each one appears here as an ABANDON line
  EVIDENCE: no work package of PLAN.md was deferred, so no gate is abandoned and this file has no ABANDON line. What RESULTS.md lists under "Deferrals and gaps" are case-level gaps (11 cases partly shown, case 13 not built); each is carried as open work in docs/research/2026-11-11-open-items/OPEN-ITEMS.md. Read this ledger's ALL MET as "every work package is merged", not as "every spec barrier is complete".
