---
name: supervisor-worker
description: >
  Handback format for a worker whose result a completion gate will check
  ({HARNESS}). Use whenever you finish delegated work and report it back:
  the first lines of your reply are a fixed header that names the verdict,
  the number of checks you ran and one line per check with its exact
  command and exit code. A check you did not run is not a check.
---

# Worker handback ({HARNESS})

The gate reads the first lines of your final message. It links every cited
check to a command it saw run. Anything else is a claim, and a claim does not
make a task ready.

## The header

Your final message starts with these lines, no code fence, nothing before them:

```text
VERDICT: PASS
CHECKS RUN: 2
CHECK C2 | cargo test -p my-crate | exit 0
CHECK C3 | cargo clippy --all-targets -- -D warnings | exit 0
```

- `VERDICT:` is one of `PASS`, `FAIL`, `UNVERIFIED`.
- `CHECKS RUN:` is the count of `CHECK` lines that follow. The gate drops the
  whole header when the count and the lines disagree.
- `CHECK <criterion id> | <exact command> | exit <code>`; add
  `| cwd <directory>` when the command ran somewhere other than the project
  root. The criterion id and the command come from the criteria table the
  chair gave you (id, command, mandatory). Run the table's command and cite it
  verbatim, character for character. A different command, even one that
  differs by a single flag, is refused and links no run. Without a table, the
  first refusal names the command each criterion requires.

After the header, write the rest of your report as usual.

## Rules the gate applies

- **Would run is not run.** A command you describe, plan, recommend or "would
  run" is not a check. Cite only commands that executed in this session.
- **UNVERIFIED is never PASS.** If you could not run the check (tool missing,
  build broken, no time), say `VERDICT: UNVERIFIED` and `CHECKS RUN: 0`.
  A `PASS` with zero checks is refused.
- **The command runs alone.** Run the table's command with nothing appended,
  piped or redirected (no `; echo $?`, no `| tail`, no `> log`). Any other
  text does not link to the criterion, and a pipeline reports the exit code
  of its last stage, not of the check.
- **A non-zero exit is a FAIL, not a note.** Report the exit code the command
  returned; do not round it to success because the failure looked unrelated.
- **The candidate is what you cite.** If you edit a file after a check ran,
  that check is stale; run it again and cite the new run.
- **A started job is not a passed check.** With Terminal Commander, supply
  an explicit absolute cwd, retain the job ID and obtain final authoritative
  status. Keep argv boundaries intact. The task, candidate and cwd recorded
  at dispatch must match the check; later polling cannot rebind an old run.
- **No prompt text, no secrets in the header.** Commands only. Redact tokens
  from a command before citing it.

## What happens next

The chair presents your handback to the gate. The gate accepts a handback whose
cited checks it observed and whose candidate is unchanged; otherwise it refuses
with a reason. A refusal is not a judgement of your work, it is a missing
receipt: run the check, cite it, hand back again.
