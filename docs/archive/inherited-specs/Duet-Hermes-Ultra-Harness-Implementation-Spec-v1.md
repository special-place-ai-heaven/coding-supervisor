# Duet on Hermes Ultra: harness implementation specification v1

Date: 23 October 2026  
Status: proposed implementation contract for the user's existing custom fork  
Inputs: *Build Your Own Duet*, @choopyplug1, 15-page supplied PDF; *Hermes Harness Research Ledger*, corrected image-link revision  
Scope: apply harness engineering to the PDF's eight-layer proactive assistant

## 1. What this specification changes

Keep the product in the PDF: one conversational surface, an attention loop that discovers what matters, and a work loop that prepares useful results. Implement durable control and verifiable outcomes beneath those loops.

The model remains responsible for contextual judgment, finding relevant relationships, selecting useful work, drafting, and solving tasks. Application code owns event identity, permissions, routing constraints, accounting, persistence, and execution status. The goal is useful initiative that survives interruption and can explain itself.

This document makes specific decisions where the PDF is illustrative or contradictory. MUST denotes a requirement of this proposed v1. Defaults are provisional configuration values for replay evaluation, not empirically established optimums. Interfaces are logical contracts to adapt to your existing code; they are not claims about existing Hermes Ultra APIs.

Your fork source was not supplied. No repository files were edited, no compatibility was established against a commit, and the acceptance scenarios below have not been executed. The PDF's claims about commercial products are not independently verified or needed to implement this design.

## 2. Source-to-implementation map

PDF references use printed page numbers. H references identify the earlier research ledger.

| Duet layer | Concrete harness addition | PDF | Ledger |
|---|---|---|---|
| Context | Versioned instructions, provenance, task state, source-health records, retrieval manifest | pp. 3, 9 | H02, H03, H06, H12 |
| Watch | Transactional intake, separate event/incident identities, replay-safe cursors, durable dispatch queue | pp. 3–4, 8 | H04, H06, H10, H12 |
| Judge | Typed judgments, verified input references, deterministic severity floors, versioned rubric | p. 4 | H07, H09, H11, H18 |
| Speak | One routing policy, notification outbox, P0 exception, explicit silence reasons | p. 5, pp. 9–10 | H04, H09, H10, H12, H13 |
| Prepare | Versioned task contract, capability limits, checkpoints, artifact-bound checks | pp. 5–6, 8 | H01, H02, H05, H06, H07, H08 |
| Gate | Immutable action packets, scoped approval, execution-time revalidation, reconciliation | pp. 6–8 | H04, H07, H09, H10 |
| Defend | Shared enforcement across all tools, credential broker, outbound inspection, tenant isolation | pp. 7, 10 | H04, H09, H11 |
| Learn | Explicit feedback semantics, candidate policy revisions, replay before activation, rollback | pp. 7–8, 12–13 | H14, H15, H17, H18, H19 |

## 3. Decisions that repair the PDF's gaps

| ID | Source issue | v1 resolution |
|---|---|---|
| D01 | Listing 4 applies the daily budget to P0, although the prose and route matrix exempt P0 (p. 5). | P0 bypasses quiet hours and the non-P0 notification cap. Incident deduplication still applies. |
| D02 | The route matrix permits urgent P1 alerts during quiet hours; the delivery code defers every non-P0 (p. 5). | Quiet hours win for P1 by default. A separately configured urgent-P1 override may permit delivery. Record the choice in policy. |
| D03 | Cursors advance to `now` after the attention pass (p. 8). Partial source failure or processing failure is unspecified. | Persist source items and the source's returned cursor atomically, per page and per source. Judge asynchronously from durable intake. |
| D04 | One hash of source/entity/kind/window is used as event identity (p. 3). | Separate transport deduplication from incident correlation. Include tenant/account scope; distinct incidents cannot collapse merely because they share a time window. |
| D05 | Four weeks of same-weekday/hour data may contain only four observations; missing/zero deviation is handled too loosely (p. 4). | Represent insufficient history explicitly; require configured sample support and a domain-specific scale floor. Do not interpret unavailable statistics as normality. |
| D06 | P0 without an evidence URL is demoted (p. 4). | Require durable evidence identity, not a publicly reachable URL. A confirmed critical signal stays critical when a link is inaccessible. Uncorroborated claims trigger enrichment rather than automatic trust. |
| D07 | The refund example presents a refund plus email under one apparent approval, while surrounding text requires separate actions (pp. 6–7). | Two immutable packets, with independent approvals. The email depends on a reconciled successful refund. |
| D08 | Implicit threshold updates execute immediately, while weekly changes are described as approval-gated (pp. 7–8). | Implicit feedback creates proposals only. Explicit user rules can activate through the authenticated control path; global/off-limits policy remains authoritative. |
| D09 | Branch pushes are described as drafts even though they can trigger CI, webhooks, or notifications (p. 8). | Classify pushes by actual effects. Permit only a scoped standing grant or an action packet; local branches need no external grant. |
| D10 | A single packet ID is treated as enough for idempotency (p. 8). | Use transactional execution claims plus target-supported idempotency/reconciliation. Preserve unknown outcomes. |
| D11 | A source with no returned events can look identical to an unavailable source (pp. 3, 5). | Brief coverage distinguishes checked-and-quiet, partially checked, unavailable, and stale. |
| D12 | Urgency words alone are rejected, but the cheat sheet uses a VIP message saying down/cancel as a P0 cue (pp. 4, 14). | Message content is evidence for a claim, not authority over severity. Corroborate impact with available sources; use an explicit uncertainty path when corroboration is unavailable. |
| D13 | One ongoing thread can be mistaken for one enormous model context (p. 3). | Preserve a unified user-facing thread; compile bounded task-specific context from durable state. |
| D14 | Watching and cost are described as almost free or flat as sources grow (pp. 8, 11). | Meter connector calls, enrichment, batching, token use, retries, and work separately. No constant-cost assumption. |

## 4. Runtime ownership and trust

Use the existing fork's scheduler, model client, tool dispatcher, persistence, and delivery adapters if suitable. Introduce the following responsibilities through adapters or extensions, not necessarily new services:

1. Intake transaction writer.
2. Context compiler and judgment validator.
3. Notification router and delivery outbox.
4. Task controller with evidence registry.
5. Action broker and packet state machine.
6. Feedback/replay evaluator.

Only authenticated user commands and versioned operator-approved policies may change controlling instructions. Trusted repository instructions may apply within an explicitly authorized coding task. Fetched emails, issues, documents, calendar descriptions, and web pages remain task data; they cannot grant authority.

Every capability entry point MUST enforce the same policy: normal tools, shell, browser, MCP/plugins, scheduled runs, delegated work, and direct provider SDK wrappers. An unrestricted network-capable shell defeats a policy applied only to named API tools.

Prepare may read authorized sources and write local artifacts. Remote drafts, branch pushes, and other external writes require explicitly permitted adapter capabilities. Connector secrets remain in the broker and outside model-visible arguments/results. Secret redaction is defense in depth; it does not replace scoped credentials and execution isolation.

Owner-directed alerts and briefs use a standing notification grant scoped to the user and configured channels. They MUST NOT require fresh approval for every alert. Customer messages, refunds, merges, deployment, signing, and deletion remain individually gated unless a later explicit policy authorizes a narrower autonomous workflow. Off-limits actions cannot be enabled by approving a packet.

## 5. Durable records and invariants

All records MUST carry `schema_version`, `tenant_id`, `record_id`, `created_at`, and version/concurrency metadata where mutable. Use UTC instants internally and an IANA timezone for local scheduling. Unknown fields from model outputs MUST be rejected at authority boundaries. Model output cannot set trusted producer identities or execution outcomes.

| Record | Required domain fields | Invariant |
|---|---|---|
| SourcePage | connection_id, cursor_before, cursor_after, fetched_at, health, item_ids | Cursor commits only with durable intake for that page. |
| Observation | connection_id, provider_event_id, provider_revision, occurred_at, observed_at, entity, kind, evidence_refs | Delivery dedupe is unique within tenant and connection, with revision where required. |
| Incident | incident_id, correlation_rule_version, signature, opened_at, last_seen, status, observation_ids | A resolved incident's recurrence is a new episode. |
| Evidence | producer, source_record, source_revision, captured_at, digest, access_scope, retention_until | Source URLs are optional navigation aids; evidence identity is durable. |
| Judgment | incident_id, incident_version, rubric_version, context_manifest_id, model_config, severity, confidence, rationale, evidence_refs, floor_results | Unknown evidence references or invalid enum values cannot become accepted judgments. |
| Decision | judgment_id, policy_version, route, reason_code, decided_at, next_review_at | Silence has a recorded reason and any required wake-up time. |
| Notification | recipient_id, channel_id, incident_id, notification_kind, escalation_generation, policy_version, payload_digest, status | One logical notification per dedupe tuple; transport uncertainty is explicit. |
| Task | contract_revision, trigger_ids, trigger_version, objective, allowed_capabilities, criteria, budget, status | Duplicate triggers cannot start duplicate active tasks for the same work purpose. |
| CheckResult | task_id, criterion_id, candidate_digest, check_version, producer, result, log_ref | Results attach to the checked candidate, not whatever files happen to exist later. |
| Packet | task_id, action_type, target, exact_payload, payload_digest, expected_target_version, dependency_ids, expires_at, status | Published packet revisions are immutable. |
| Approval | packet_id, packet_revision, payload_digest, authenticated_actor, decision, decided_at | Approval of an old revision cannot authorize an edited payload. |
| Execution | packet_id, attempt_id, effect_key, expected_target_version, lease_epoch, status, remote_operation_id, evidence_refs | A dispatch with uncertain outcome cannot be treated as safely retryable failure. |
| Feedback | subject_type, subject_id, reason, actor, timestamp | Packet rejection and alert relevance are distinct signals. |
| PolicyRevision | predecessor, exact_user_rules, compiled_rules, compiler_version, replay_result, activation_status | Model-inferred preferences cannot silently overwrite explicit rules. |

Evidence storage MUST minimize retained sensitive payloads. Keep a protected snapshot or a structured attestation sufficient to support the claim, according to retention policy. A hash alone does not establish what a source said; a URL alone does not guarantee reproducibility.

## 6. Watch: concrete intake algorithm

Connector interface:

```text
fetch_page(connection, cursor) ->
  items[], next_cursor, has_more, source_watermark, health
```

For each connection independently:

```text
1. Acquire a bounded connection lease; load its committed cursor.
2. Fetch one page outside the database transaction.
3. Validate item structure and connection/tenant ownership.
4. In one transaction:
   a. Insert unseen observations using provider identity/revision.
   b. Upsert incident membership using the configured correlation rule.
   c. Insert durable judge-queue entries for newly relevant versions.
   d. Compare-and-swap cursor_before -> next_cursor.
   e. Record source health and fetched watermark.
5. Commit; continue if has_more and intake budget remains.
6. Release the lease. Judge consumes persisted queue entries separately.
```

A malformed item MUST be quarantined durably with a failure record before the connector may advance past it. If a trustworthy envelope cannot be retained, do not advance; expose the intake failure. One source failing does not prevent successfully ingested sources from committing.

Timestamp-only connectors MUST use a configured overlap interval and stable item dedupe. Never use the scheduler's `now` as an assumed safe source watermark. Push handlers acknowledge only after durable intake. Webhook authentication occurs before intake.

Transport duplicates do not increment occurrence count. Actual repeated error occurrences do. If the provider supplies aggregate counters, its adapter computes increments from ordered versions and detects counter resets; summing snapshots is forbidden.

Incident grouping defaults to provider incident IDs when available. Otherwise use a versioned signature including tenant, account, service/entity, environment, and error fingerprint. Time windows bound correlation; they are not the only identity input. Severity increase, recurrence after resolution, and material scope expansion are notification-relevant changes.

## 7. Judge: where the model's intelligence belongs

### Candidate selection

Evaluate deterministic critical signals before the model prefilter. Candidate set is the union of:

- Critical/service-specific floor triggers.
- Configured always-salient event kinds.
- First-seen or insufficient-history events.
- Statistical anomalies above a configured threshold.
- Deadline, unanswered-request, ownership, or relationship changes that need contextual judgment.

The last category is necessary: a meeting conflict or a customer waiting for a promised export can matter without an error spike. Do not restrict the assistant to anomaly detection.

Baseline returns `FIRST_SEEN`, `INSUFFICIENT_HISTORY`, `NOT_APPLICABLE`, or a numeric score with sample count and baseline version. Provisional baseline: eight matching weekly/hourly buckets, minimum six valid buckets, domain-defined scale floor. Exclude confirmed incidents, incomplete buckets, and the current observation from the reference sample. Low-volume counts/rates need an adapter-specific estimator; no universal z-score assumption.

### Bounded contextual judgment

Compile a manifest containing the incident, applicable exact rules, ownership, linked changes, relevant deadlines, previous decisions, source health, and evidence references. Fetch additional material only to answer a concrete unresolved question. Log retrieval reason and source revision.

The model returns exactly:

```json
{
  "incident_id": "inc-checkout-001",
  "severity": "P0",
  "confidence": "high",
  "impact": 3,
  "money": 2,
  "deadline_at": null,
  "person_waiting": false,
  "rationale": "Confirmed production checkout failures after the latest deploy.",
  "evidence_refs": ["ev-monitor-001", "ev-deploy-001"],
  "suggested_work": "prepare_rollback_assessment",
  "missing_information": []
}
```

Allowed enums: severity `P0|P1|P2|noise`; confidence `low|medium|high`; scores are integers 0–3. Work suggestions come from an allowlist. Scores inform review but do not substitute for a documented severity rubric.

Validate output, resolve references within the tenant, and apply deterministic floors. A low-confidence model cannot lower a known critical floor. Untrusted urgency words cannot create one.

Malformed output gets at most one repair call per batch. After another failure, persist `judgment_unavailable`: deliver known critical signals through the deterministic route; queue the remainder for bounded retry and show degraded triage coverage. Never quietly classify them as noise.

Batch size and token limits MUST be explicit. Split oversized batches; do not truncate away incidents. Isolate unrelated tenant/customer contexts, and validate every returned incident ID. Correlation suggestions from a model do not merge incident identities without validated matching rules.

## 8. Speak: one authoritative routing function

Provisional defaults: Europe/Ljubljana, brief at 08:00, quiet hours 22:00–07:30, three non-P0 interruptions per local day, urgent P1 window three hours, P1 follow-up after four hours. These values are configuration, not extracted user preferences. Travel information may propose a timezone change; it must not silently activate one.

```text
route(judgment, incident, policy, now):
  if incident resolved: return LOG_ONLY("resolved")
  if equivalent notification already confirmed:
      return UPDATE_EXISTING("already_notified")
  if severity == P0:
      return ALERT_NOW("critical_floor_or_validated_P0")

  wants_alert = severity == P1 and (
      deadline within urgent_window or escalation_timer_due)
  if wants_alert and quiet_now and not urgent_P1_quiet_override:
      return DEFER("quiet_hours", next_quiet_end)
  if wants_alert and non_P0_budget_available:
      return ALERT_NOW("urgent_or_followup")
  if wants_alert:
      return PENDING_MERGE("attention_budget", next_brief_or_budget_reset)
  if severity == P1: return BRIEF_TOP("P1")
  if severity == P2: return BRIEF_LINE("P2")
  return LOG_ONLY("noise")
```

`PENDING_MERGE` MUST have a scheduled next evaluation; it cannot wait forever for another alert to happen. At reevaluation, refresh resolution, severity, ownership, deadline, and authorization before delivery. If a deadline passes while deferred, report it as missed/expired context; never silently act to compensate.

For P0, duplicate occurrences update the existing incident view without another buzz. Material escalation or a new episode may generate a new notification. Abuse-resistant incident correlation must coexist with the P0 cap exemption.

Budget reservation and notification outbox creation occur in one transaction. Concurrent workers cannot both spend the last slot. Confirmed delivery commits the spend; confirmed no-delivery releases it; uncertain delivery retains the reservation pending reconciliation. Use the transport's idempotency key if supported. If delivery status is unknowable and the transport has no dedupe, do not promise exactly-once notification; log uncertainty and apply an explicitly configured retry/escalation policy.

Daily budget keys include recipient, local date, and timezone-policy version. A timezone change must not grant an accidental second daily allowance. Repeated DST times produce one brief; nonexistent local times use the next valid instant. Persist the selected brief instant before dispatch.

The brief MUST distinguish:

- New items from already-alerted updates.
- Verified work ready for review from work still running or blocked.
- Checked-and-quiet sources from unavailable, stale, or incomplete sources.
- Raw observations from grouped incidents and suppressed notifications.

Full packets appear in the authenticated interface. Lock-screen previews omit amounts, full customer identity, and draft text by default. Outbound policy applies to alerts, briefs, packets, logs exposed to users, and third-party messages.

## 9. Prepare: task contract, completion, and resume

A background trigger may enqueue preparation automatically when a standing preparation policy covers its task kind and capabilities. The system should prepare useful work without asking whether it may begin every time. Unclear priorities can be recorded as assumptions; missing consequential authority blocks only the affected action.

Example concrete coding contract:

```yaml
schema_version: 1
task_id: task-ci-001
trigger: {incident_id: inc-ci-001, version: 3}
objective: Prepare a tested fix for the failing main-branch tax test
workspace: {repository_id: repo-api, base_commit: recorded-at-start}
capabilities: [repo_read, local_edit, local_test]
forbidden: [merge, deploy, external_message]
output: [patch, test_evidence, explanation]
criteria:
  - id: C1
    check: baseline_reproduces_reported_failure
  - id: C2
    check: candidate_passes_reported_test
  - id: C3
    check: configured_regression_suite_passes
  - id: C4
    check: diff_within_contract_scope
budget: {max_model_turns: 40, max_wall_seconds: 7200, max_repair_rounds: 3}
on_block: checkpoint_and_report
```

Runtime resolves `recorded-at-start` to an immutable commit before execution. Each configured check MUST exist in a trusted registry. The model cannot invent a check name or replace a check with a self-reported pass.

Task states: `queued -> preparing -> verifying -> ready_for_review`; verification failure may enter `repairing -> verifying`. Other terminal/paused states: `blocked`, `cancelled`, `budget_exhausted`, `failed`. `ready_for_review` requires all mandatory checks passing on the current candidate. It does not mean a proposed external action has executed.

Store checks with candidate digest, contract revision, command/check version, environment identity, start/end timestamps, exit/status, and retained result. v1 conservatively invalidates all code-check results whenever candidate content changes; dependency-aware reuse is a later optimization. Adding a receipt outside candidate content does not invalidate code checks.

Resume loads the contract, task projection, last committed event sequence, candidate identity, pending/unknown actions, and remaining budget. It reobserves the workspace before mutating. Never mark work complete from a conversation summary. Child calls inherit shared task limits; restarting does not reset consumed budgets.

Preparation may finish even while an approval packet is pending. Separate task readiness from packet execution. A stopped or budget-exhausted task produces a partial receipt with missing criteria and a resume reference, not a generic success message.

## 10. Gate: exact action, exact approval, known outcome

### Packet lifecycle

States: `draft`, `awaiting_approval`, `approved`, `executing`, `succeeded`, `rejected`, `expired`, `superseded`, `failed_no_effect`, `unknown`.

| Transition | Required guard |
|---|---|
| draft -> awaiting_approval | Preparation evidence current; target ownership verified; policy permits; payload complete. |
| awaiting_approval -> approved | Authenticated authorized actor; current revision/digest; not expired. |
| awaiting_approval -> rejected | Authenticated skip/reject with optional reason. |
| awaiting_approval/approved -> expired | Current time reaches expiry before dispatch. |
| awaiting_approval/approved -> superseded | Payload edited, target version changes materially, or policy revision invalidates permission. |
| approved -> executing | Transactional execution claim; current policy, expiry, dependencies, target version and payload digest rechecked. |
| executing -> succeeded | Adapter supplies verifiable confirmation of the intended effect. |
| executing -> failed_no_effect | Adapter establishes that no effect occurred. |
| executing -> unknown | Timeout/crash/ambiguous response after possible dispatch. |
| unknown -> succeeded/failed_no_effect | Reconciliation establishes the result. |

Editing creates a new packet revision and invalidates prior approval. Rejecting a packet does not necessarily resolve its triggering incident.

Compute the payload digest from a canonical encoding of action type, exact target, payload, attachments/diff digests, and dependencies. The UI displays the same immutable representation the broker consumes. Do not execute regenerated model text after approval.

### Execution algorithm

```text
1. Authenticate approval and persist its exact packet binding.
2. Claim the approved packet using compare-and-swap and a lease epoch.
3. Recheck current policy, expiry, dependency outcomes and target version.
4. Persist execution intent and stable effect_key before dispatch.
5. Broker calls the adapter with expected version and effect_key.
6. Persist succeeded, failed_no_effect, or unknown with evidence.
7. Render the outcome from the persisted result.
```

Use provider conditional writes when supported. A local preflight read alone cannot prevent the target changing between the read and write. Where the provider lacks conditional execution, document the residual race and refuse high-consequence actions whose required precondition cannot be enforced adequately.

Lease expiry after a dispatch does not permit a new worker to issue the action again. It enters reconciliation. A local lock cannot guarantee remote exactly-once effects.

### Refund example

- Packet A: refund payment `ch_8812`, exact amount/currency, customer account, expected refundable balance.
- Packet B: send the exact customer email; dependency is A's confirmed success.
- A and B have separate controls. B cannot send merely because A was approved.
- If A times out, the refund adapter queries the operation/idempotency key before deciding whether another dispatch is safe.
- Any rewritten email becomes a new B revision. If a prepared email states a completed refund, it remains blocked until that statement is supported.

## 11. Learn: improve judgment without learning the wrong lesson

Do not use one `skip` flag for everything. Capture separate events:

| Feedback | Meaning | Allowed learning proposal |
|---|---|---|
| Alert irrelevant | Wrong subject, owner, or importance | Scoped relevance/ownership adjustment |
| Alert mistimed | Relevant but poorly timed | Delivery timing adjustment |
| Duplicate alert | Already known | Correlation/delivery fix |
| Packet rejected | User does not want this action | Action-preference review, not automatic alert suppression |
| Packet edited | Draft content changed | Scoped drafting preference with the edit diff |
| Incident acknowledged | User has seen it | Stop redundant reminders; incident may remain open |
| Explicit instruction | Direct authenticated user rule | Versioned rule change within the user's authority |

Implicit feedback is evidence for a candidate change, not an automatic live threshold update. Candidate changes include scope, before/after behavior, supporting examples, replay outcome, and rollback reference. Hold critical floors and off-limits capabilities outside learned adjustments.

Replay tests judgment policy separately from model variability. Store input snapshots/manifests, decision times, rubric versions, model settings, and output records. Deterministic routing replay uses recorded judgments; judgment evaluation reruns the model on frozen cases and reports variance. Do not leak future outcomes or later user preferences into historical decisions.

Promote only if predefined quality constraints pass and the applicable user/operator policy authorizes activation. A direct user request to quiet a defined class is itself authorization for the applicable change; do not create unnecessary approval loops.

## 12. Worked lifecycle: useful initiative with enforced boundaries

This is a synthetic example, not an observed run of your fork.

| Time | Input/state | Required behavior |
|---|---|---|
| 01:05 | Confirmed production checkout failure; daily non-P0 cap already exhausted | Persist incident and evidence; apply P0 floor; enqueue owner alert immediately. |
| 01:05 | Preparation grant covers incident analysis | Start a bounded timeline/rollback-assessment task without waiting for another user answer. |
| 01:06 | Eleven observations of the same unresolved incident | Update count and timeline; no eleven extra alerts or tasks. |
| 01:08 | Rollback candidate prepared with current deployment reference | Run registered preflight checks; create exact rollback packet only if policy permits it. |
| 01:10 | User approves packet revision 1 | Bind approval to revision 1 and its digest. |
| 01:10 | Deployment changed after packet was created | Revalidation supersedes packet; prepare a refreshed candidate. Old approval cannot run it. |
| 01:12 | User approves refreshed revision; remote response times out | Mark unknown; query deployment operation state; do not repeat blindly. |
| 01:13 | Reconciliation confirms rollback | Record success evidence; send an authorized owner update. |
| 08:00 | Morning brief; inbox connector was unavailable overnight | List incident as an update, distinguish rollback execution from service recovery, show inbox coverage as unavailable. |
| Later | User rejects a customer apology draft | Record packet feedback; do not lower the checkout incident's criticality. |

Rollback success does not itself prove service recovery. A separate service check supplies that evidence and closes the incident only when its resolution policy is met.

## 13. Acceptance scenarios for the implementing agent

These are test specifications to implement against your fork, not tests already run. Use fake adapters and controlled clocks before enabling live delivery.

| ID | Given / when | Required assertion |
|---|---|---|
| A01 | P0 arrives during quiet hours after three non-P0 alerts | One critical notification is queued; non-P0 cap does not block it. |
| A02 | Urgent P1 arrives during quiet hours; override false | Defer until quiet end with a persisted wake-up; no immediate delivery. |
| A03 | Same case; explicit override true and budget available | Immediate notification is permitted. |
| A04 | Two delivery workers compete for one remaining slot | At most one new non-P0 interruption reserves it. |
| A05 | P1 exceeds budget and no later event arrives | Scheduled reevaluation/brief still surfaces it. |
| A06 | Connector dies before intake transaction commits | Cursor unchanged; retry retains all source items without double counting. |
| A07 | Process dies after intake commits but before Judge runs | Durable queue resumes judgment; cursor need not roll back. |
| A08 | One connector fails; another succeeds | Successful source advances; brief marks failed source unavailable. |
| A09 | Forty transport retries of one provider event | One observation; occurrence count unchanged by retransmission. |
| A10 | Forty genuine occurrences of one incident | Correct count; one incident; one initial notification. |
| A11 | Incident resolves and recurs | New episode may notify; old dedupe key does not suppress it. |
| A12 | New/low-support signal | Baseline unavailable state; no fabricated numeric score or silent noise label. |
| A13 | Inbox says URGENT and requests a refund; no payment record | No critical floor or refund authority is created from the message. |
| A14 | Confirmed critical telemetry lacks a public URL | Critical status remains supported by internal evidence. |
| A15 | Triage returns unknown evidence IDs or tenant data | Validation fails; no accepted judgment or cross-tenant disclosure. |
| A16 | Model call fails on a batch containing a known critical signal | Critical route proceeds; remaining triage is visibly degraded. |
| A17 | Sandbox tries a disallowed effect through shell/browser/plugin | Shared policy/execution boundary blocks it. |
| A18 | Model says tests passed without running them | Task cannot become ready_for_review. |
| A19 | Candidate changes after a passing check | Previous check is stale; readiness blocked until rechecked. |
| A20 | Worker dies mid-task and resumes | Same contract, accounted budget, pending effects and candidate identity restored. |
| A21 | Approval is replayed concurrently | At most one dispatch claim; repeated approval cannot create a second effect. |
| A22 | Packet expires or payload changes before approval | No dispatch; refresh/new revision required. |
| A23 | Target changes after approval | Conditional write fails or revalidation blocks; old action is not applied blindly. |
| A24 | Remote action succeeds and its response is lost | Unknown -> reconciliation -> succeeded; no duplicate action. |
| A25 | Refund approved but email not approved | Refund only; no email. |
| A26 | Email approved but refund outcome unknown | Email remains blocked on dependency. |
| A27 | Five critical alert skips or a rejected remediation | Critical floor remains; feedback retains its distinct meaning. |
| A28 | Candidate policy improves noise but misses a labeled critical case | Promotion fails. |
| A29 | Shadow/replay runs any scenario above | No production write or real notification path is available. |
| A30 | A brief is generated across a repeated DST hour | One brief for the intended schedule occurrence. |
| A31 | Source health is stale and there are no fetched events | Brief does not claim checked-and-quiet. |
| A32 | User asks why an event was not reported | Response cites its persisted decision and coverage state, not an invented recollection. |

Quality measurements: critical recall on labeled cases, alert precision, missed deadlines, user interruptions, false task readiness, duplicate effects, uncertain outcomes, task completion within budget, and resource use per accepted outcome. Zero failures in a finite replay suite is a release criterion, not a production guarantee.

## 14. Implementation order and fork inspection

### First useful slice

Implement one source -> durable event -> Judge -> shadow notification -> bounded local preparation -> evidence -> approval packet. Use one developer workflow already available in your fork, such as a CI failure. Keep the executor in simulated mode until packet and reconciliation tests pass.

This demonstrates proactive usefulness and reliability together. Building eight independent placeholder modules is not sufficient.

### Repository mapping required before editing

The implementing agent MUST locate and cite exact files/symbols for:

- Scheduled run entry and overlapping-run protection.
- Context construction, memory, compaction, and resume.
- Tool registration and the shared dispatcher, including bypasses.
- Model client and usage accounting.
- State database and transaction boundaries.
- Connector cursor ownership and webhook intake.
- User notification delivery and transport retry behavior.
- Authenticated UI actions for approval/edit/skip.
- Sandbox credentials and network permissions.
- Existing tests, replay fixtures, feature flags, and migration practices.

Record existing/partial/missing for each requirement. Reuse existing machinery where it meets the contract. If the fork cannot enforce the proposed capability boundary, identify that as a blocker for unattended external effects rather than pretending a prompt solves it.

### Milestones

1. **Durable shadow attention:** A01–A16, A29–A32; source health and reasons for silence visible.
2. **Verifiable preparation:** A17–A20; local artifact and check receipt ready for review.
3. **Controlled effects:** A21–A26 against fake adapters, then one explicitly enabled real action class with reconciliation support.
4. **Calibrated learning:** A27–A28, frozen replay cases, policy rollback, cost/attention reporting.

No feature is complete merely because a prompt mentions it. Completion requires behavior through the real application path and evidence from the corresponding acceptance tests.

## 15. Copy-ready implementation instruction

> Implement the Duet-specific requirements in this document inside our existing custom Hermes Ultra fork. First pin the commit and map the real scheduler, context, tools, storage, delivery, approval UI and sandbox paths. Produce a short gap matrix with code references. Preserve the eight-layer product and two-loop behavior. Start with one end-to-end shadow workflow, then add verifiable preparation and controlled execution. Resolve D01–D14 as specified unless our explicit product policy conflicts; record any proposed deviation before implementation. Use application-enforced state and permissions, preserve unknown effects, and do not invent Hermes APIs. Implement the applicable A01–A32 tests with fake sources, deterministic clocks and failure injection. Report tested behavior, exact evidence and remaining limitations. Extend existing abstractions before adding competing systems.

## 16. Source and decision record

- **Primary product design:** *Build Your Own Duet*, @choopyplug1, supplied PDF, 15 pages; read in full, including schemas, listings, appendices, and the two-loop diagram. The PDF explicitly identifies its architecture and numerical choices as the author's independent design.
- **Harness research input:** *Hermes-Harness-Research-Ledger-2026-10-23.md*, existing saved working copy with corrected image-viewer links. H01–H19 provide the traceability identifiers used here.
- **New design decisions:** D01–D14, record requirements, algorithms, proposed defaults, packet lifecycle, and A01–A32 are this specification's proposals for your implementation. They are not claims about Anthropic internals or existing Hermes Ultra functionality.
- **Boundary:** no fork source audit, executable implementation, live action, benchmark result, or production approval is claimed by this document.
