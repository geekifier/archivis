use archivis_core::models::ResolutionOutcome;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PlannedField {
    Title,
    Subtitle,
    Description,
    PublicationYear,
    Language,
    PageCount,
    Authors,
    Identifiers,
    Series,
    Cover,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldClass {
    CoreIdentity,
    Enrichment,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldDecision {
    Apply,
    Preserve,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldReason {
    Apply,
    MetadataLocked,
    Protected,
    StrongIdRequired,
    TitleContradiction,
    NoChange,
    NoCandidateValue,
    ExistingValuePreferred,
    PreserveExistingCover,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CoreFieldInput {
    pub proposed: bool,
    pub differs: bool,
    pub protected: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct EnrichmentFieldInput {
    pub proposed: bool,
    pub should_apply_if_unlocked: bool,
    pub protected: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ReconciliationInput {
    pub metadata_locked: bool,
    pub has_strong_id_proof: bool,
    pub has_title_contradiction: bool,
    pub title: CoreFieldInput,
    pub subtitle: EnrichmentFieldInput,
    pub description: EnrichmentFieldInput,
    pub publication_year: EnrichmentFieldInput,
    pub language: EnrichmentFieldInput,
    pub page_count: EnrichmentFieldInput,
    pub authors: CoreFieldInput,
    pub identifiers: EnrichmentFieldInput,
    pub series: CoreFieldInput,
    pub cover: EnrichmentFieldInput,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FieldAction {
    pub field: PlannedField,
    pub class: FieldClass,
    pub decision: FieldDecision,
    pub reason: FieldReason,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReconciliationPlan {
    pub outcome: ResolutionOutcome,
    pub should_apply_candidate: bool,
    pub field_actions: Vec<FieldAction>,
}
impl std::fmt::Display for PlannedField {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Title => write!(f, "title"),
            Self::Subtitle => write!(f, "subtitle"),
            Self::Description => write!(f, "description"),
            Self::PublicationYear => write!(f, "publication_year"),
            Self::Language => write!(f, "language"),
            Self::PageCount => write!(f, "page_count"),
            Self::Authors => write!(f, "authors"),
            Self::Identifiers => write!(f, "identifiers"),
            Self::Series => write!(f, "series"),
            Self::Cover => write!(f, "cover"),
        }
    }
}

/// Extract human-readable dispute reasons from a reconciliation plan.
///
/// Returns one entry per field that was preserved due to a conflict
/// (protection, lock, contradiction, or weak ID).
pub fn extract_dispute_reasons(plan: &ReconciliationPlan) -> Vec<String> {
    plan.field_actions
        .iter()
        .filter_map(|action| {
            if action.decision != FieldDecision::Preserve {
                return None;
            }
            let label = field_display_label(action.field);
            match action.reason {
                FieldReason::TitleContradiction => {
                    Some(format!("{label} differs from provider's suggestion"))
                }
                FieldReason::Protected => Some(format!(
                    "{label} is protected and won't be changed automatically"
                )),
                FieldReason::MetadataLocked => {
                    Some(format!("{label} update blocked — metadata is locked"))
                }
                FieldReason::StrongIdRequired => Some(format!(
                    "{label} change skipped — no strong identifier match"
                )),
                _ => None,
            }
        })
        .collect()
}

fn field_display_label(field: PlannedField) -> &'static str {
    match field {
        PlannedField::Title => "Title",
        PlannedField::Subtitle => "Subtitle",
        PlannedField::Description => "Description",
        PlannedField::PublicationYear => "Publication year",
        PlannedField::Language => "Language",
        PlannedField::PageCount => "Page count",
        PlannedField::Authors => "Authors",
        PlannedField::Identifiers => "Identifiers",
        PlannedField::Series => "Series",
        PlannedField::Cover => "Cover",
    }
}

impl ReconciliationPlan {
    pub fn should_apply(&self, field: PlannedField) -> bool {
        self.field_actions
            .iter()
            .any(|action| action.field == field && action.decision == FieldDecision::Apply)
    }
}

pub fn plan_reconciliation(input: &ReconciliationInput) -> ReconciliationPlan {
    let mut field_actions = Vec::with_capacity(10);

    let mut has_core_conflict = false;
    let mut has_core_change = false;
    let mut has_enrichment_change = false;
    let mut has_candidate_change = false;

    for (field, data) in [
        (PlannedField::Title, input.title),
        (PlannedField::Authors, input.authors),
        (PlannedField::Series, input.series),
    ] {
        let action = plan_core_field(field, data, input);
        if data.differs {
            has_candidate_change = true;
        }
        if action.decision == FieldDecision::Apply {
            has_core_change = true;
        } else if data.proposed && data.differs {
            has_core_conflict = true;
        }
        field_actions.push(action);
    }

    for (field, data) in [
        (PlannedField::Subtitle, input.subtitle),
        (PlannedField::Description, input.description),
        (PlannedField::PublicationYear, input.publication_year),
        (PlannedField::Language, input.language),
        (PlannedField::PageCount, input.page_count),
        (PlannedField::Identifiers, input.identifiers),
        (PlannedField::Cover, input.cover),
    ] {
        let action = plan_enrichment_field(field, data, input.metadata_locked);
        if data.should_apply_if_unlocked {
            has_candidate_change = true;
        }
        if action.decision == FieldDecision::Apply {
            has_enrichment_change = true;
        }
        field_actions.push(action);
    }

    let identity_confirmed = input.has_strong_id_proof && !has_core_conflict;
    let should_apply_candidate = !input.metadata_locked
        && !has_core_conflict
        && (has_core_change || has_enrichment_change || identity_confirmed);
    let has_dispute = has_core_conflict || (input.metadata_locked && has_candidate_change);

    let outcome = if has_dispute {
        ResolutionOutcome::Disputed
    } else if has_core_change {
        ResolutionOutcome::Confirmed
    } else if has_enrichment_change {
        ResolutionOutcome::Enriched
    } else if identity_confirmed {
        ResolutionOutcome::Confirmed
    } else {
        ResolutionOutcome::Enriched
    };

    ReconciliationPlan {
        outcome,
        should_apply_candidate,
        field_actions,
    }
}

pub fn plan_automatic_reconciliation(input: &ReconciliationInput) -> ReconciliationPlan {
    plan_reconciliation(input)
}

fn plan_core_field(
    field: PlannedField,
    data: CoreFieldInput,
    input: &ReconciliationInput,
) -> FieldAction {
    let reason = if !data.proposed {
        FieldReason::NoCandidateValue
    } else if !data.differs {
        FieldReason::NoChange
    } else if input.metadata_locked {
        FieldReason::MetadataLocked
    } else if data.protected {
        FieldReason::Protected
    } else if !input.has_strong_id_proof {
        FieldReason::StrongIdRequired
    } else if input.has_title_contradiction {
        FieldReason::TitleContradiction
    } else {
        FieldReason::Apply
    };

    let decision = if reason == FieldReason::Apply {
        FieldDecision::Apply
    } else {
        FieldDecision::Preserve
    };

    FieldAction {
        field,
        class: FieldClass::CoreIdentity,
        decision,
        reason,
    }
}

fn plan_enrichment_field(
    field: PlannedField,
    data: EnrichmentFieldInput,
    metadata_locked: bool,
) -> FieldAction {
    let reason = if !data.proposed {
        FieldReason::NoCandidateValue
    } else if metadata_locked {
        FieldReason::MetadataLocked
    } else if !data.should_apply_if_unlocked {
        // Check applicability BEFORE protection: if the field wouldn't be
        // applied anyway (existing value preferred), don't flag as Protected.
        if field == PlannedField::Cover {
            FieldReason::PreserveExistingCover
        } else {
            FieldReason::ExistingValuePreferred
        }
    } else if data.protected {
        FieldReason::Protected
    } else {
        FieldReason::Apply
    };

    let decision = if reason == FieldReason::Apply {
        FieldDecision::Apply
    } else {
        FieldDecision::Preserve
    };

    FieldAction {
        field,
        class: FieldClass::Enrichment,
        decision,
        reason,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protected_core_difference_is_disputed() {
        let plan = plan_reconciliation(&ReconciliationInput {
            has_strong_id_proof: true,
            title: CoreFieldInput {
                proposed: true,
                differs: true,
                protected: true,
            },
            ..ReconciliationInput::default()
        });

        assert_eq!(plan.outcome, ResolutionOutcome::Disputed);
        assert!(!plan.should_apply_candidate);
        assert_eq!(plan.field_actions[0].decision, FieldDecision::Preserve);
        assert_eq!(plan.field_actions[0].reason, FieldReason::Protected);
    }

    #[test]
    fn enrichment_only_plan_is_enriched() {
        let plan = plan_reconciliation(&ReconciliationInput {
            has_strong_id_proof: true,
            description: EnrichmentFieldInput {
                proposed: true,
                should_apply_if_unlocked: true,
                protected: false,
            },
            ..ReconciliationInput::default()
        });

        assert_eq!(plan.outcome, ResolutionOutcome::Enriched);
        assert!(plan.should_apply_candidate);
        assert!(plan.should_apply(PlannedField::Description));
    }

    #[test]
    fn strong_id_core_change_is_confirmed() {
        let plan = plan_reconciliation(&ReconciliationInput {
            has_strong_id_proof: true,
            title: CoreFieldInput {
                proposed: true,
                differs: true,
                protected: false,
            },
            ..ReconciliationInput::default()
        });

        assert_eq!(plan.outcome, ResolutionOutcome::Confirmed);
        assert!(plan.should_apply_candidate);
        assert!(plan.should_apply(PlannedField::Title));
    }

    #[test]
    fn extract_disputes_from_locked_book() {
        let plan = plan_reconciliation(&ReconciliationInput {
            metadata_locked: true,
            has_strong_id_proof: true,
            title: CoreFieldInput {
                proposed: true,
                differs: true,
                protected: false,
            },
            description: EnrichmentFieldInput {
                proposed: true,
                should_apply_if_unlocked: true,
                protected: false,
            },
            ..ReconciliationInput::default()
        });

        let reasons = extract_dispute_reasons(&plan);
        assert!(!reasons.is_empty());
        assert!(reasons.iter().any(|r| r.contains("metadata is locked")));
    }

    #[test]
    fn extract_disputes_from_protected_title() {
        let plan = plan_reconciliation(&ReconciliationInput {
            has_strong_id_proof: true,
            title: CoreFieldInput {
                proposed: true,
                differs: true,
                protected: true,
            },
            ..ReconciliationInput::default()
        });

        let reasons = extract_dispute_reasons(&plan);
        assert!(!reasons.is_empty());
        assert!(reasons.iter().any(|r| r.contains("protected")));
    }

    #[test]
    fn extract_disputes_empty_when_no_conflicts() {
        let plan = plan_reconciliation(&ReconciliationInput {
            has_strong_id_proof: true,
            description: EnrichmentFieldInput {
                proposed: true,
                should_apply_if_unlocked: true,
                protected: false,
            },
            ..ReconciliationInput::default()
        });

        let reasons = extract_dispute_reasons(&plan);
        assert!(reasons.is_empty());
    }

    #[test]
    fn extract_disputes_weak_id() {
        let plan = plan_reconciliation(&ReconciliationInput {
            has_strong_id_proof: false,
            title: CoreFieldInput {
                proposed: true,
                differs: true,
                protected: false,
            },
            ..ReconciliationInput::default()
        });

        let reasons = extract_dispute_reasons(&plan);
        assert!(reasons
            .iter()
            .any(|r| r.contains("no strong identifier match")));
    }

    #[test]
    fn locked_book_stays_disputed() {
        let plan = plan_reconciliation(&ReconciliationInput {
            metadata_locked: true,
            has_strong_id_proof: true,
            description: EnrichmentFieldInput {
                proposed: true,
                should_apply_if_unlocked: true,
                protected: false,
            },
            ..ReconciliationInput::default()
        });

        assert_eq!(plan.outcome, ResolutionOutcome::Disputed);
        assert!(!plan.should_apply_candidate);
    }

    #[test]
    fn enrichment_protected_with_existing_value_not_disputed() {
        let plan = plan_reconciliation(&ReconciliationInput {
            has_strong_id_proof: true,
            language: EnrichmentFieldInput {
                proposed: true,
                should_apply_if_unlocked: false,
                protected: true,
            },
            ..ReconciliationInput::default()
        });

        let lang_action = plan
            .field_actions
            .iter()
            .find(|a| a.field == PlannedField::Language)
            .unwrap();
        assert_eq!(
            lang_action.reason,
            FieldReason::ExistingValuePreferred,
            "protected enrichment with existing value should be ExistingValuePreferred, not Protected"
        );

        let reasons = extract_dispute_reasons(&plan);
        assert!(
            !reasons.iter().any(|r| r.contains("Language")),
            "should not generate a dispute for Language when existing value is preferred"
        );
    }

    #[test]
    fn enrichment_protected_without_existing_value_still_disputed() {
        let plan = plan_reconciliation(&ReconciliationInput {
            has_strong_id_proof: true,
            description: EnrichmentFieldInput {
                proposed: true,
                should_apply_if_unlocked: true,
                protected: true,
            },
            ..ReconciliationInput::default()
        });

        let desc_action = plan
            .field_actions
            .iter()
            .find(|a| a.field == PlannedField::Description)
            .unwrap();
        assert_eq!(
            desc_action.reason,
            FieldReason::Protected,
            "protected enrichment that would apply should still be Protected"
        );

        let reasons = extract_dispute_reasons(&plan);
        assert!(
            reasons.iter().any(|r| r.contains("Description")),
            "should generate a dispute for Description when protection blocks a real enrichment"
        );
    }
}
