const CHAIR_TEMPLATE: &str = include_str!("../../skills/supervisor-chair.SKILL.template.md");

#[test]
fn chair_guidance_requires_explicit_task_activation() {
    let normalized = CHAIR_TEMPLATE
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    assert!(normalized.contains("Installation does not activate a task."));
    assert!(normalized.contains("supervisor task start"));
    assert!(
        !normalized
            .contains("the task exists as soon as the contract is in the gate's state directory")
    );
}
