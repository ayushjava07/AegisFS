//! Comprehensive unit and boundary tests for the audit logging subsystem.

use std::collections::BTreeMap;

use super::*;
use crate::domain::ids::{generate_id, AuditRecordId};

#[test]
fn audit_record_builder_and_accessors() {
    let id = AuditRecordId::parse(&generate_id("au_")).unwrap();
    let record = AuditRecord::new(
        id.clone(),
        1700000000000,
        AuditActor::User {
            username: "operator".into(),
            role: "admin".into(),
        },
        AuditAction::WorkflowCreated {
            name: "daily-backup".into(),
            version: 1,
        },
        AuditOutcome::Success,
        "workflow",
    )
    .with_tenant("acme-corp")
    .with_resource_id("wf_daily_backup")
    .with_meta("cluster", "us-east-1");

    assert_eq!(record.id, id);
    assert_eq!(record.timestamp_ms, 1700000000000);
    assert_eq!(record.tenant.as_deref(), Some("acme-corp"));
    assert_eq!(record.resource_type, "workflow");
    assert_eq!(record.resource_id.as_deref(), Some("wf_daily_backup"));
    assert_eq!(record.actor.kind_str(), "user");
    assert_eq!(record.action.action_name(), "workflow_created");
    assert_eq!(record.outcome.status_str(), "success");
    assert!(record.outcome.is_success());
    assert_eq!(
        record.metadata.get("cluster").and_then(|v| v.as_str()),
        Some("us-east-1")
    );
}

#[test]
fn audit_actor_and_action_discriminators() {
    let sys = AuditActor::System {
        component: "scheduler".into(),
    };
    assert_eq!(sys.kind_str(), "system");

    let tok = AuditActor::Token {
        token_id: "tk_123".into(),
        tenant_id: "acme".into(),
        role: "operator".into(),
    };
    assert_eq!(tok.kind_str(), "token");

    let anon = AuditActor::Anonymous {
        client_ip: Some("127.0.0.1".into()),
    };
    assert_eq!(anon.kind_str(), "anonymous");

    let denied = AuditOutcome::Denied {
        reason: "insufficient permissions".into(),
    };
    assert_eq!(denied.status_str(), "denied");
    assert!(!denied.is_success());

    let err = AuditOutcome::Error {
        message: "disk full".into(),
    };
    assert_eq!(err.status_str(), "error");
    assert!(!err.is_success());

    assert_eq!(
        AuditAction::WorkflowDeleted {
            name: "test".into()
        }
        .action_name(),
        "workflow_deleted"
    );
    assert_eq!(
        AuditAction::RunSubmitted {
            run_id: "rn_1".into(),
            workflow_name: "test".into(),
        }
        .action_name(),
        "run_submitted"
    );
    assert_eq!(
        AuditAction::RunCancelled {
            run_id: "rn_1".into(),
            reason: "manual".into(),
        }
        .action_name(),
        "run_cancelled"
    );
    assert_eq!(
        AuditAction::AuthFailed {
            reason: "bad sig".into(),
            client_ip: None,
        }
        .action_name(),
        "auth_failed"
    );
    assert_eq!(
        AuditAction::TokenCreated {
            token_id: "tk_1".into(),
            role: "admin".into(),
        }
        .action_name(),
        "token_created"
    );
    assert_eq!(
        AuditAction::TokenRevoked {
            token_id: "tk_1".into(),
        }
        .action_name(),
        "token_revoked"
    );
    assert_eq!(
        AuditAction::LeaseReaped { count: 3 }.action_name(),
        "lease_reaped"
    );
    assert_eq!(
        AuditAction::RetentionPurged { count: 42 }.action_name(),
        "retention_purged"
    );
}

#[test]
fn audit_filter_combinators() {
    let id = AuditRecordId::parse(&generate_id("au_")).unwrap();
    let record = AuditRecord {
        id,
        timestamp_ms: 500,
        tenant: Some("t1".into()),
        actor: AuditActor::Token {
            token_id: "tk_abc".into(),
            tenant_id: "t1".into(),
            role: "operator".into(),
        },
        action: AuditAction::RunCancelled {
            run_id: "rn_xyz".into(),
            reason: "stopped".into(),
        },
        outcome: AuditOutcome::Success,
        resource_type: "run".into(),
        resource_id: Some("rn_xyz".into()),
        metadata: BTreeMap::new(),
    };

    // Matching filters
    assert!(AuditFilter::new().matches(&record));
    assert!(AuditFilter::new().with_tenant("t1").matches(&record));
    assert!(AuditFilter::new().with_actor_kind("token").matches(&record));
    assert!(AuditFilter::new()
        .with_action_name("run_cancelled")
        .matches(&record));
    assert!(AuditFilter::new().with_status("success").matches(&record));
    assert!(AuditFilter::new()
        .with_resource_type("run")
        .matches(&record));
    assert!(AuditFilter::new()
        .with_time_range(Some(400), Some(600))
        .matches(&record));

    // Non-matching filters
    assert!(!AuditFilter::new().with_tenant("other").matches(&record));
    assert!(!AuditFilter::new().with_actor_kind("user").matches(&record));
    assert!(!AuditFilter::new()
        .with_action_name("run_submitted")
        .matches(&record));
    assert!(!AuditFilter::new().with_status("error").matches(&record));
    assert!(!AuditFilter::new()
        .with_resource_type("workflow")
        .matches(&record));
    assert!(!AuditFilter::new()
        .with_time_range(Some(600), None)
        .matches(&record));
    assert!(!AuditFilter::new()
        .with_time_range(None, Some(400))
        .matches(&record));
}

#[test]
fn audit_record_json_round_trip() {
    let id = AuditRecordId::parse(&generate_id("au_")).unwrap();
    let record = AuditRecord::new(
        id,
        123456789,
        AuditActor::Anonymous {
            client_ip: Some("192.168.1.1".into()),
        },
        AuditAction::AuthFailed {
            reason: "invalid signature".into(),
            client_ip: Some("192.168.1.1".into()),
        },
        AuditOutcome::Denied {
            reason: "bad credentials".into(),
        },
        "auth",
    );

    let json = serde_json::to_string(&record).expect("serialize");
    let deserialized: AuditRecord = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(deserialized, record);
}
