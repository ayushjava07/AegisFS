//! Automated validation test suite for example workflows.
//!
//! Asserts that all shipped example workflow definitions parse cleanly,
//! conform to structural validation rules, have valid topological orderings,
//! and produce zero static analysis warnings during dry-run simulation.

use std::fs;
use std::path::Path;

use runvane::api::payloads::WorkflowSpec;
use runvane::engine::dry_run::simulate_workflow;

#[test]
fn validate_shipped_example_workflows() {
    let examples_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/workflows");
    assert!(
        examples_dir.exists(),
        "examples/workflows directory must exist"
    );

    let entries = fs::read_dir(&examples_dir).expect("cannot read examples/workflows directory");
    let mut validated_count = 0;

    for entry in entries {
        let entry = entry.expect("valid directory entry");
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) == Some("json") {
            let content = fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));

            let spec: WorkflowSpec = serde_json::from_str(&content)
                .unwrap_or_else(|e| panic!("failed to parse {}: {e}", path.display()));

            // Instantiate into formal domain definition
            let def = spec
                .into_definition(1_000)
                .unwrap_or_else(|e| panic!("invalid definition in {}: {e}", path.display()));

            // Execute dry-run simulation and verify absence of dangling references/warnings
            let report = simulate_workflow(&def)
                .unwrap_or_else(|e| panic!("simulation failed for {}: {e}", path.display()));

            assert!(
                report.total_tasks > 0,
                "workflow in {} must define at least one task",
                path.display()
            );
            assert!(
                !report.stages.is_empty(),
                "workflow in {} must have at least one execution stage",
                path.display()
            );
            assert!(
                report.warnings.is_empty(),
                "workflow in {} produced static warnings: {:?}",
                path.display(),
                report.warnings
            );

            validated_count += 1;
        }
    }

    assert!(
        validated_count >= 3,
        "expected at least 3 example workflows validated, found {validated_count}"
    );
}
