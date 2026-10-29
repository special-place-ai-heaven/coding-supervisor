// Throwaway capture hook for the OMP T3 investigation. Not an adapter.
//
// Every event below appends its payload (strings truncated to 300 characters,
// credential-like keys redacted) to captures/<session>.events.jsonl, then
// applies the decision configured in hooks/policy.json for the test named by
// the OMP_T3_TEST environment variable.

import { createHash } from "node:crypto";
import { appendFileSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { basename, dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const FIXTURE =
	process.env.OMP_T3_FIXTURE ?? join(dirname(fileURLToPath(import.meta.url)), "..", "..", "..");
const CAPTURES = join(FIXTURE, "captures");
const TRUNCATE = 300;
const SECRET_KEY = /^(api[_-]?key|apikey|authorization|password|cookie|token|access[_-]?token|refresh[_-]?token|.*secret.*|.*credential.*)$/i;

function trunc(v: unknown, depth = 0, seen = new WeakSet<object>()): unknown {
	if (typeof v === "string") return v.length <= TRUNCATE ? v : `${v.slice(0, TRUNCATE)}...[+${v.length - TRUNCATE}]`;
	if (typeof v === "function") return "[fn]";
	if (typeof v === "bigint") return String(v);
	if (v === null || typeof v !== "object") return v;
	if (seen.has(v)) return "[circular]";
	seen.add(v);
	if (depth > 6) return "[depth]";
	let out: unknown;
	if (Array.isArray(v)) {
		const arr = v.slice(0, 20).map(x => trunc(x, depth + 1, seen));
		if (v.length > 20) arr.push(`[+${v.length - 20} items]`);
		out = arr;
	} else {
		const o: Record<string, unknown> = {};
		for (const [k, x] of Object.entries(v)) o[k] = SECRET_KEY.test(k) ? "[redacted]" : trunc(x, depth + 1, seen);
		out = o;
	}
	seen.delete(v); // only true cycles are cut; shared references are written twice
	return out;
}

function load(path: string, fallback: any): any {
	try {
		return JSON.parse(readFileSync(path, "utf8"));
	} catch {
		return fallback;
	}
}

function digest(): string {
	try {
		return createHash("sha256").update(readFileSync(join(FIXTURE, "candidate.txt"))).digest("hex").slice(0, 16);
	} catch {
		return "missing";
	}
}

function gateState(): string {
	return load(join(FIXTURE, "gate-state.json"), {}).state ?? "unknown";
}

function textOf(content: unknown): string {
	if (typeof content === "string") return content;
	if (!Array.isArray(content)) return "";
	return content.map((b: any) => (b && b.type === "text" ? String(b.text ?? "") : "")).join("\n");
}

function claimsDone(text: string): boolean {
	// Known limit (T3 lesson): a word match, not a parse; negations are excluded crudely.
	return /\b(done|complete|completed|finished)\b/i.test(text) &&
		!/\b(not|isn't|cannot|can't|unable|refus\w*|incomplete|withdraw\w*)\b/i.test(text);
}

export default function capture(pi: any): void {
	const test = process.env.OMP_T3_TEST ?? "default";
	const policies = load(join(FIXTURE, "hooks", "policy.json"), {});
	const policy = policies[test] ?? policies.default ?? {};
	const state = { lastText: "", stopBlocks: 0 };

	function sessionId(ctx: any): string {
		try {
			const sm = ctx?.sessionManager;
			const id = sm?.getSessionId?.();
			if (id) return String(id);
			const file = sm?.getSessionFile?.();
			if (file) return basename(String(file)).replace(/\.jsonl$/, "");
		} catch {}
		return "nosession";
	}

	function record(event: string, payload: any, ctx: any, decision: unknown, extra: Record<string, unknown> = {}) {
		const sid = sessionId(ctx);
		const line = {
			t: new Date().toISOString(),
			test,
			event,
			session: sid,
			agent: trunc(ctx?.agent),
			mode: ctx?.mode,
			keys: payload && typeof payload === "object" ? Object.keys(payload).sort() : [],
			payload: trunc(payload),
			candidate_digest_now: digest(),
			decision: decision ?? null,
			...extra,
		};
		mkdirSync(CAPTURES, { recursive: true });
		appendFileSync(join(CAPTURES, `${sid}.events.jsonl`), `${JSON.stringify(line)}\n`);
	}

	for (const name of [
		"session_start",
		"session_shutdown",
		"input",
		"before_agent_start",
		"agent_start",
		"agent_end",
		"turn_start",
		"turn_end",
	]) {
		pi.on(name, async (event: any, ctx: any) => {
			record(name, event, ctx, null);
		});
	}

	pi.on("before_subagent_spawn", async (event: any, ctx: any) => {
		const decision = policy.block_spawn ? { block: true, reason: "GATE: spawn refused by fixture policy." } : undefined;
		record("before_subagent_spawn", event, ctx, decision);
		return decision;
	});

	pi.on("tool_call", async (event: any, ctx: any) => {
		if (policy.throw_tool_call && event?.toolName === policy.throw_tool_call) {
			record("tool_call", event, ctx, { threw: "GATE fixture: deliberate handler error" });
			throw new Error("GATE fixture: deliberate handler error");
		}
		let decision: unknown;
		const input = JSON.stringify(event?.input ?? {});
		if (policy.block_out_txt && /out\.txt/.test(input) && ["write", "edit", "bash"].includes(event?.toolName)) {
			decision = {
				block: true,
				reason: `GATE: ${event.toolName} to out.txt refused; gate state is ${gateState()} and no current receipt exists.`,
			};
		}
		record("tool_call", event, ctx, decision);
		return decision;
	});

	pi.on("tool_result", async (event: any, ctx: any) => {
		let decision: unknown;
		const extra: Record<string, unknown> = {};
		// Worker output reaches the parent in the task result (blocking agents) or in the wait result (async).
		if ((event?.toolName === "task" || event?.toolName === "wait") && ctx?.agent?.kind !== "sub" && /VERDICT|CHECKS RUN/.test(textOf(event.content))) {
			const text = textOf(event.content);
			const checksRun = Number(/CHECKS RUN:\s*(\d+)/.exec(text)?.[1] ?? NaN);
			const cited = /CHECK \S+ \| .+ \| exit \d+/.test(text);
			const unsupported = !cited || checksRun === 0;
			if (/VERDICT: PASS/.test(text)) {
				const receipt = { candidate_digest: digest(), claim: "VERDICT: PASS", checks_run: checksRun, cited, session: sessionId(ctx) };
				writeFileSync(join(CAPTURES, `${sessionId(ctx)}.receipt.json`), JSON.stringify(receipt));
				extra.receipt = receipt;
			}
			const reason = "GATE: worker claim is unsupported (no check ran, no exit code cited); not accepted.";
			if (unsupported && policy.task_result_mode === "annotate") decision = { additionalContext: reason };
			if (unsupported && policy.task_result_mode === "replace") decision = { content: [{ type: "text", text: `[GATE] ${reason}` }] };
		}
		record("tool_result", event, ctx, decision, extra);
		return decision;
	});

	pi.on("assistant_message", async (event: any, ctx: any) => {
		const content = event?.message?.content;
		const text = textOf(content);
		if (ctx?.agent?.kind !== "sub" && text.trim()) state.lastText = text;
		let decision: unknown;
		if (policy.replace_final && ctx?.agent?.kind !== "sub" && claimsDone(text) && Array.isArray(content)) {
			decision = {
				content: content.map((b: any) =>
					b?.type === "text" ? { ...b, text: `[GATE: completion claim not accepted] ${b.text}` } : b,
				),
			};
		}
		record("assistant_message", event, ctx, decision, { claims_done: claimsDone(text) });
		return decision;
	});

	pi.on("session_stop", async (event: any, ctx: any) => {
		const last =
			[textOf(event?.last_assistant_message?.content), event?.last_assistant_message].find(
				x => typeof x === "string" && x.trim(),
			) ?? state.lastText;
		const sid = sessionId(ctx);
		const receipt = load(join(CAPTURES, `${sid}.receipt.json`), null);
		const now = digest();
		const stale = receipt !== null && receipt.candidate_digest !== now;
		const reasons: string[] = [];
		const gs = gateState();
		if (gs !== "completed") reasons.push(`gate state is ${gs}, not completed`);
		if (stale) reasons.push(`receipt is stale: candidate ${receipt.candidate_digest} != now ${now}`);
		if (receipt === null) reasons.push("no receipt recorded");
		const claims = claimsDone(last);
		const max = policy.stop_block_max ?? 1;
		let decision: unknown;
		if (policy.stop_mode === "block" && (claims || policy.stop_block_always) && reasons.length && state.stopBlocks < max) {
			state.stopBlocks++;
			decision = { decision: "block", reason: `GATE refuses completion: ${reasons.join("; ")}` };
		}
		record("session_stop", event, ctx, decision, {
			gate: { state: gs, claims_done: claims, stale, reasons, last_text: trunc(last), stop_blocks: state.stopBlocks },
		});
		return decision;
	});
}
