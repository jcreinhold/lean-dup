use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::retrieval::{CandidateSet, KeyContribution, RetrievedCandidate};
use crate::review_policy;
use crate::scorer;
use crate::semantic_reranking::SearchSemanticObligationFact;
use crate::semantic_verification::{EvidenceKind, EvidenceStatus, SemanticEvidence};
use crate::source_refs::{ImportStatus, SourceFacts};
use lean_dup_index::HydratedDeclaration;
use lean_dup_index::{ComparisonEvidenceMode, ComparisonEvidencePolicy};
use lean_dup_worker::SourceSpan;

/// Policy input for turning retrieved candidates into review groups.
///
/// Ranking consumes typed semantic and source facts. It does not query storage,
/// run probes, parse worker transport records, parse CLI flags, or render text.
#[derive(Debug, Clone)]
pub struct RankingInput<'a> {
    pub candidate_sets: &'a [CandidateSet],
    pub semantic_evidence: &'a BTreeMap<String, SemanticEvidence>,
    pub source_facts: &'a SourceFacts,
    pub profile: RankingProfile,
    pub comparison_policy: &'a ComparisonEvidencePolicy,
}

/// Tunable ranking defaults owned by search policy, not by CLI parsing.
#[derive(Debug, Clone, Copy)]
pub struct RankingProfile {
    pub min_near_score: f64,
    pub transitional_alias_callers: usize,
}

impl Default for RankingProfile {
    fn default() -> Self {
        let thresholds = scorer::thresholds();
        Self {
            min_near_score: thresholds.near_score,
            transitional_alias_callers: thresholds.transitional_alias_callers,
        }
    }
}

/// Ranked review queue plus diagnostics that later renderers can present.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RankedReview {
    pub groups: Vec<RankedGroup>,
    pub suppressed: Vec<SuppressedGroup>,
    pub diagnostics: RankingDiagnostics,
}

impl RankedReview {
    pub fn visible_groups(&self, filter: ReviewFilter) -> Vec<&RankedGroup> {
        self.groups.iter().filter(|group| filter.includes(group)).collect()
    }
}

/// User-facing visibility intent for audit queues.
#[derive(Debug, Clone, Copy)]
pub struct ReviewFilter {
    pub include_generated: bool,
    pub include_private: bool,
    pub include_diagnostics: bool,
    pub min_priority: ReviewPriority,
}

impl ReviewFilter {
    pub fn includes(self, group: &RankedGroup) -> bool {
        if group.review_priority == ReviewPriority::Noise && !self.include_diagnostics {
            return false;
        }
        if !self.include_generated && group.blockers.iter().any(|blocker| blocker == "generated-declaration") {
            return false;
        }
        if !self.include_private && group.members.iter().any(|member| member.visibility != "public") {
            return false;
        }
        if !self.include_diagnostics
            && group
                .blockers
                .iter()
                .any(|blocker| matches!(blocker.as_str(), "lean-probe-rejected" | "lean-probe-unavailable"))
        {
            return false;
        }
        group.review_priority <= self.min_priority
    }
}

/// One ranked candidate group ready for review or rendering.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RankedGroup {
    pub id: String,
    pub pair_id: String,
    pub relation: ReviewRelation,
    pub members: Vec<ReviewMember>,
    pub evidence: Vec<ReviewEvidence>,
    pub signals: Vec<String>,
    pub blockers: Vec<String>,
    pub confidence: ConfidenceTier,
    pub review_priority: ReviewPriority,
    pub recommended_action: ReviewAction,
    pub target_decl: Option<String>,
    pub target_module: Option<String>,
    pub evidence_mode: ReviewEvidenceMode,
    pub probe_summary: Option<String>,
    pub semantic_obligations: Vec<SearchSemanticObligationFact>,
    pub local_caller_count: usize,
    pub replacement_hint: Option<crate::replacement_hints::ReplacementHint>,
}

/// Whether a ranked group rests on static or proof-grade comparison evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ReviewEvidenceMode {
    Static,
    SourceBackedNotImportable,
    ProofGrade,
}

/// Declaration facts exposed to review output.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReviewMember {
    pub declaration_id: String,
    pub origin: String,
    pub module: String,
    pub qualified_name: String,
    pub display_name: String,
    pub kind: String,
    pub visibility: String,
    pub source_span: Option<SourceSpan>,
    pub status_flags: Vec<String>,
}

/// One typed evidence item supporting a ranked group.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ReviewEvidence {
    pub kind: String,
    pub role: Option<String>,
    pub display: Option<String>,
    pub score: f64,
}

impl ReviewEvidence {
    pub fn summary(&self) -> String {
        let role = self.role.as_deref().unwrap_or("-");
        let display = self.display.as_deref().unwrap_or("-");
        format!(
            "evidence={} role={} display={} score={:.3}",
            self.kind, role, display, self.score
        )
    }
}

/// Relation selected as the strongest explanation for a group.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ReviewRelation {
    ExactStatement,
    PermutedStatement,
    ConnectiveEquivalent,
    Specialization,
    SourceClone,
    SubsumptionCandidate,
    NearStatement,
}

/// Recommended cleanup or review action.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ReviewAction {
    AlreadyInMathlib,
    LocalAlias,
    ReplaceLocalUses,
    InlinePrivateHelper,
    MergeGeneralization,
    SpecializationOf,
    ProbableSourceClone,
    ManualReview,
}

/// Confidence tier for queue ordering and default filtering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ConfidenceTier {
    High,
    Medium,
    Low,
    Noise,
}

/// Review priority. Lower variants sort earlier and are included by stricter filters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ReviewPriority {
    High,
    Medium,
    Low,
    Noise,
}

/// A weaker relation hidden because a stronger relation covers the same pair.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SuppressedGroup {
    pub pair_id: String,
    pub suppressed_relation: ReviewRelation,
    pub covered_by: ReviewRelation,
    pub reason: String,
}

/// Ranking counters for diagnostics and later `show` support.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct RankingDiagnostics {
    pub candidate_pairs: usize,
    pub emitted_groups: usize,
    pub suppressed_groups: usize,
}

/// Rank retrieved candidates into actionable review groups.
pub fn rank_candidates(input: RankingInput<'_>) -> RankedReview {
    let mut groups = Vec::new();
    let mut suppressed = Vec::new();
    let mut seen_group_ids = BTreeSet::new();

    for set in input.candidate_sets {
        for candidate in &set.candidates {
            let group = rank_pair(&set.anchor, candidate, &input);
            suppressed.extend(suppressed_relations(&group, candidate));
            if seen_group_ids.insert(group.id.clone()) {
                groups.push(group);
            }
        }
    }

    groups.sort_by(|left, right| {
        left.review_priority
            .cmp(&right.review_priority)
            .then_with(|| left.confidence.cmp(&right.confidence))
            .then_with(|| left.id.cmp(&right.id))
    });

    let diagnostics = RankingDiagnostics {
        candidate_pairs: input.candidate_sets.iter().map(|set| set.candidates.len()).sum(),
        emitted_groups: groups.len(),
        suppressed_groups: suppressed.len(),
    };

    RankedReview {
        groups,
        suppressed,
        diagnostics,
    }
}

fn rank_pair(anchor: &HydratedDeclaration, candidate: &RetrievedCandidate, input: &RankingInput<'_>) -> RankedGroup {
    let semantic = input.semantic_evidence.get(&candidate.pair_id);
    let mut signals = contribution_signals(&candidate.explanation.contributions);
    let mut blockers = BTreeSet::new();
    let source_clone = same_source_fingerprint(anchor, &candidate.declaration, input.source_facts);
    let has_mathlib = candidate.declaration.origin == "mathlib" || anchor.origin == "mathlib";
    let evidence_mode = review_evidence_mode(anchor, &candidate.declaration, semantic, input.comparison_policy);
    let theorem_pair = theorem_like(anchor) && theorem_like(&candidate.declaration);
    let verified_exact = verified_kind(semantic, EvidenceKind::ExactTheorem)
        || verified_kind(semantic, EvidenceKind::ReducibleDefinition);
    let verified_specialization = verified_kind(semantic, EvidenceKind::Specialization);
    let verified_permuted = verified_kind(semantic, EvidenceKind::PermutedTheorem);
    let verified_replacement = verified_kind(semantic, EvidenceKind::Replacement);
    let semantic_required = input.comparison_policy.requires_semantic_evidence(&anchor.origin)
        || input
            .comparison_policy
            .requires_semantic_evidence(&candidate.declaration.origin);
    let static_evidence_allowed = !semantic_required;
    let score_facts = scorer::ranking_score_facts(candidate, theorem_pair, static_evidence_allowed);
    let exact = verified_exact || score_facts.exact_static;
    let specialization = theorem_pair && verified_specialization;
    let permuted = verified_permuted || score_facts.permuted_static;
    let connective = verified_replacement || score_facts.connective_static;
    let near = score_facts.near;

    if let Some(semantic) = semantic {
        signals.extend(semantic_signals(semantic));
        match semantic.status {
            EvidenceStatus::Unavailable => {
                blockers.insert("lean-probe-unavailable".to_owned());
            }
            EvidenceStatus::Rejected => {
                blockers.insert("lean-probe-rejected".to_owned());
            }
            EvidenceStatus::Verified => {}
        }
    }
    if semantic_required
        && score_facts.strong_static_semantic_candidate
        && semantic.is_none_or(|semantic| !semantic.proof_grade())
    {
        blockers.insert("unverified-proof-grade-evidence".to_owned());
    }
    if source_clone {
        signals.insert("source-clone".to_owned());
        blockers.remove("non-theorem-static-only");
    }
    if verified_exact || specialization || verified_permuted || verified_replacement {
        blockers.remove("non-theorem-static-only");
    }
    blockers.extend(review_policy::visibility_blockers(
        anchor,
        &candidate.declaration,
        &candidate.explanation.contributions,
    ));
    if !exact && !permuted && !connective && !specialization && !source_clone && !near {
        blockers.insert("weak-feature-overlap".to_owned());
    }

    let relation = if exact {
        ReviewRelation::ExactStatement
    } else if specialization {
        ReviewRelation::Specialization
    } else if permuted {
        ReviewRelation::PermutedStatement
    } else if connective {
        ReviewRelation::ConnectiveEquivalent
    } else if source_clone {
        ReviewRelation::SourceClone
    } else if near {
        ReviewRelation::SubsumptionCandidate
    } else {
        ReviewRelation::NearStatement
    };
    let target = recommended_target(anchor, &candidate.declaration);
    let local_caller_count = local_caller_count(anchor, &candidate.declaration, input.source_facts);
    let import_status = target
        .as_ref()
        .and_then(|target| target_module(target))
        .map(|target_module| local_import_status(anchor, &candidate.declaration, target_module, input.source_facts));
    let mut priority = priority_for(
        relation,
        has_mathlib,
        &blockers,
        candidate.score,
        input.profile.min_near_score,
    );
    let mut confidence = confidence_for(relation, priority);
    if blockers.contains("generated-declaration")
        || blockers.contains("broad-head-only")
        || blockers.contains("low-signal-declaration")
        || blockers.contains("typeclass-instance-noise")
        || blockers.contains("non-theorem-static-only")
        || blockers.contains("unverified-proof-grade-evidence")
    {
        priority = ReviewPriority::Noise;
        confidence = ConfidenceTier::Noise;
    }
    let recommended_action = action_for(
        relation,
        has_mathlib,
        local_caller_count,
        import_status,
        input.profile.transitional_alias_callers,
    );

    let evidence = review_evidence(&candidate.explanation.contributions);

    RankedGroup {
        id: stable_group_id(anchor, &candidate.declaration, relation),
        pair_id: candidate.pair_id.clone(),
        relation,
        members: vec![member(anchor), member(&candidate.declaration)],
        evidence,
        signals: signals.into_iter().collect(),
        blockers: blockers.into_iter().collect(),
        confidence,
        review_priority: priority,
        recommended_action,
        target_decl: target.as_ref().map(|declaration| declaration.qualified_name.clone()),
        target_module: target.as_ref().map(|declaration| declaration.module.clone()),
        evidence_mode,
        probe_summary: semantic.and_then(semantic_summary),
        semantic_obligations: semantic
            .map(|evidence| vec![evidence.semantic_obligation_fact()])
            .unwrap_or_default(),
        local_caller_count,
        replacement_hint: None,
    }
}

fn review_evidence_mode(
    anchor: &HydratedDeclaration,
    candidate: &HydratedDeclaration,
    semantic: Option<&SemanticEvidence>,
    policy: &ComparisonEvidencePolicy,
) -> ReviewEvidenceMode {
    if semantic.is_some_and(SemanticEvidence::proof_grade) {
        return ReviewEvidenceMode::ProofGrade;
    }
    let anchor_mode = policy.evidence_mode(&anchor.origin);
    let candidate_mode = policy.evidence_mode(&candidate.origin);
    if anchor_mode == ComparisonEvidenceMode::SourceBackedNotImportable
        || candidate_mode == ComparisonEvidenceMode::SourceBackedNotImportable
    {
        ReviewEvidenceMode::SourceBackedNotImportable
    } else if anchor_mode == ComparisonEvidenceMode::ProofGrade || candidate_mode == ComparisonEvidenceMode::ProofGrade
    {
        ReviewEvidenceMode::ProofGrade
    } else {
        ReviewEvidenceMode::Static
    }
}

fn stable_group_id(anchor: &HydratedDeclaration, candidate: &HydratedDeclaration, relation: ReviewRelation) -> String {
    let mut member_ids = vec![anchor.declaration_id.as_str(), candidate.declaration_id.as_str()];
    member_ids.sort();
    let relation = match relation {
        ReviewRelation::ExactStatement => "exact-statement",
        ReviewRelation::PermutedStatement => "permuted-statement",
        ReviewRelation::ConnectiveEquivalent => "connective-equivalent",
        ReviewRelation::Specialization => "specialization",
        ReviewRelation::SourceClone => "source-clone",
        ReviewRelation::SubsumptionCandidate => "subsumption-candidate",
        ReviewRelation::NearStatement => "near-statement",
    };
    let encoded = serde_json::to_vec(&(relation, member_ids)).expect("stable group id ingredients serialize");
    let digest = Sha256::digest(&encoded);
    let suffix = digest
        .iter()
        .take(6)
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("{relation}-{suffix}")
}

fn review_evidence(contributions: &[KeyContribution]) -> Vec<ReviewEvidence> {
    contributions
        .iter()
        .map(|contribution| ReviewEvidence {
            kind: contribution.kind.clone(),
            role: contribution.role.clone(),
            display: contribution.display.clone(),
            score: contribution.score,
        })
        .collect()
}

fn action_for(
    relation: ReviewRelation,
    has_mathlib: bool,
    local_caller_count: usize,
    import_status: Option<ImportStatus>,
    transitional_alias_callers: usize,
) -> ReviewAction {
    match relation {
        ReviewRelation::ExactStatement if has_mathlib => ReviewAction::AlreadyInMathlib,
        ReviewRelation::ExactStatement => {
            if local_caller_count > 0
                && local_caller_count < transitional_alias_callers
                && import_status != Some(ImportStatus::Missing)
            {
                ReviewAction::ReplaceLocalUses
            } else {
                ReviewAction::LocalAlias
            }
        }
        ReviewRelation::PermutedStatement | ReviewRelation::ConnectiveEquivalent => ReviewAction::LocalAlias,
        ReviewRelation::Specialization => ReviewAction::SpecializationOf,
        ReviewRelation::SourceClone => ReviewAction::ProbableSourceClone,
        ReviewRelation::SubsumptionCandidate => ReviewAction::MergeGeneralization,
        ReviewRelation::NearStatement => ReviewAction::ManualReview,
    }
}

fn priority_for(
    relation: ReviewRelation,
    has_mathlib: bool,
    blockers: &BTreeSet<String>,
    score: f64,
    min_near_score: f64,
) -> ReviewPriority {
    if blockers.contains("weak-feature-overlap") || score < min_near_score {
        return ReviewPriority::Low;
    }
    match relation {
        ReviewRelation::ExactStatement if has_mathlib => ReviewPriority::High,
        ReviewRelation::ExactStatement => ReviewPriority::High,
        ReviewRelation::PermutedStatement | ReviewRelation::ConnectiveEquivalent => ReviewPriority::Medium,
        ReviewRelation::Specialization => ReviewPriority::Medium,
        ReviewRelation::SourceClone => ReviewPriority::Low,
        ReviewRelation::SubsumptionCandidate if has_mathlib => ReviewPriority::Low,
        ReviewRelation::SubsumptionCandidate => ReviewPriority::Medium,
        ReviewRelation::NearStatement => ReviewPriority::Low,
    }
}

fn confidence_for(relation: ReviewRelation, priority: ReviewPriority) -> ConfidenceTier {
    if priority == ReviewPriority::Noise {
        return ConfidenceTier::Noise;
    }
    match relation {
        ReviewRelation::ExactStatement => ConfidenceTier::High,
        ReviewRelation::PermutedStatement
        | ReviewRelation::ConnectiveEquivalent
        | ReviewRelation::Specialization
        | ReviewRelation::SourceClone
        | ReviewRelation::SubsumptionCandidate => ConfidenceTier::Medium,
        ReviewRelation::NearStatement => ConfidenceTier::Low,
    }
}

fn contribution_signals(contributions: &[KeyContribution]) -> BTreeSet<String> {
    contributions
        .iter()
        .map(|contribution| contribution.kind.clone())
        .collect()
}

fn semantic_signals(evidence: &SemanticEvidence) -> BTreeSet<String> {
    let mut signals = BTreeSet::new();
    let status = match evidence.status {
        EvidenceStatus::Verified => "verified",
        EvidenceStatus::Unavailable => "unavailable",
        EvidenceStatus::Rejected => "rejected",
    };
    let kind = match evidence.kind {
        EvidenceKind::ExactTheorem => "exact-theorem",
        EvidenceKind::PermutedTheorem => "permuted-theorem",
        EvidenceKind::Replacement => "replacement",
        EvidenceKind::ReducibleDefinition => "reducible-definition",
        EvidenceKind::Specialization => "specialization",
        EvidenceKind::LocalDuplicate => "local-duplicate",
        EvidenceKind::Unavailable => "unavailable",
    };
    signals.insert(format!("probe:{status}:{kind}"));
    if evidence.proof_grade() {
        match evidence.kind {
            EvidenceKind::ExactTheorem => {
                signals.insert("probe:same-statement".to_owned());
            }
            EvidenceKind::PermutedTheorem => {
                signals.insert("probe:same-up-to-safe-reordering".to_owned());
            }
            EvidenceKind::Replacement => {
                signals.insert("probe:replacement".to_owned());
            }
            EvidenceKind::ReducibleDefinition => {
                signals.insert("probe:same-reducible-definition".to_owned());
            }
            EvidenceKind::Specialization => {
                signals.insert("probe:specialization".to_owned());
            }
            EvidenceKind::LocalDuplicate | EvidenceKind::Unavailable => {}
        }
    }
    signals
}

fn verified_kind(evidence: Option<&SemanticEvidence>, kind: EvidenceKind) -> bool {
    evidence.is_some_and(|evidence| evidence.kind == kind && evidence.proof_grade())
}

fn same_source_fingerprint(left: &HydratedDeclaration, right: &HydratedDeclaration, facts: &SourceFacts) -> bool {
    let Some(left) = facts.source_fingerprint(&left.declaration_id) else {
        return false;
    };
    let Some(right) = facts.source_fingerprint(&right.declaration_id) else {
        return false;
    };
    !left.is_empty() && left == right
}

fn theorem_like(declaration: &HydratedDeclaration) -> bool {
    review_policy::theorem_like(declaration)
}

fn recommended_target<'a>(
    anchor: &'a HydratedDeclaration,
    candidate: &'a HydratedDeclaration,
) -> Option<&'a HydratedDeclaration> {
    let priority = |declaration: &HydratedDeclaration| match declaration.origin.as_str() {
        "mathlib" => 0,
        "workspace" => 2,
        _ => 1,
    };
    if (priority(anchor), &anchor.qualified_name) <= (priority(candidate), &candidate.qualified_name) {
        Some(anchor)
    } else {
        Some(candidate)
    }
}

fn target_module(declaration: &HydratedDeclaration) -> Option<&str> {
    if declaration.module.is_empty() {
        None
    } else {
        Some(&declaration.module)
    }
}

fn local_caller_count(anchor: &HydratedDeclaration, candidate: &HydratedDeclaration, facts: &SourceFacts) -> usize {
    [anchor, candidate]
        .into_iter()
        .filter(|declaration| declaration.origin == "workspace")
        .map(|declaration| facts.caller_count(&declaration.declaration_id))
        .sum()
}

fn local_import_status(
    anchor: &HydratedDeclaration,
    candidate: &HydratedDeclaration,
    target_module: &str,
    facts: &SourceFacts,
) -> ImportStatus {
    let statuses = [anchor, candidate]
        .into_iter()
        .filter(|declaration| declaration.origin == "workspace")
        .map(|declaration| facts.import_status_for(&declaration.declaration_id, target_module))
        .collect::<Vec<_>>();
    if statuses.is_empty() {
        ImportStatus::Unknown
    } else if statuses.iter().all(|status| *status == ImportStatus::Direct) {
        ImportStatus::Direct
    } else if statuses.contains(&ImportStatus::Missing) {
        ImportStatus::Missing
    } else {
        ImportStatus::Unknown
    }
}

fn member(declaration: &HydratedDeclaration) -> ReviewMember {
    ReviewMember {
        declaration_id: declaration.declaration_id.clone(),
        origin: declaration.origin.clone(),
        module: declaration.module.clone(),
        qualified_name: declaration.qualified_name.clone(),
        display_name: declaration.display_name.clone(),
        kind: declaration.kind.clone(),
        visibility: declaration.visibility.clone(),
        source_span: declaration.source_span.clone(),
        status_flags: declaration.status_flags.clone(),
    }
}

fn semantic_summary(evidence: &SemanticEvidence) -> Option<String> {
    evidence.summary.clone().or_else(|| match evidence.status {
        EvidenceStatus::Verified => Some("Lean verified semantic evidence".to_owned()),
        EvidenceStatus::Unavailable => Some("Lean probe unavailable".to_owned()),
        EvidenceStatus::Rejected => Some("Lean probe rejected the planned obligation".to_owned()),
    })
}

fn suppressed_relations(group: &RankedGroup, candidate: &RetrievedCandidate) -> Vec<SuppressedGroup> {
    if !matches!(
        group.relation,
        ReviewRelation::ExactStatement | ReviewRelation::Specialization
    ) {
        return Vec::new();
    }
    let mut suppressed = Vec::new();
    for contribution in &candidate.explanation.contributions {
        let relation = match contribution.kind.as_str() {
            "safe-permutation-fingerprint" => Some(ReviewRelation::PermutedStatement),
            "connective-fingerprint" => Some(ReviewRelation::ConnectiveEquivalent),
            "conclusion-fingerprint" => Some(ReviewRelation::SubsumptionCandidate),
            "role-feature" => Some(ReviewRelation::NearStatement),
            _ => None,
        };
        if let Some(relation) = relation
            && relation != group.relation
        {
            suppressed.push(SuppressedGroup {
                pair_id: group.pair_id.clone(),
                suppressed_relation: relation,
                covered_by: group.relation,
                reason: "covered by stronger evidence for the same pair".to_owned(),
            });
        }
    }
    suppressed.sort_by_key(|item| item.suppressed_relation);
    suppressed.dedup_by_key(|item| item.suppressed_relation);
    suppressed
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::{
        RankingInput, RankingProfile, ReviewAction, ReviewEvidenceMode, ReviewFilter, ReviewPriority, ReviewRelation,
        rank_candidates,
    };
    use crate::retrieval::{CandidateExplanation, CandidateSet, KeyContribution, RetrievedCandidate};
    use crate::semantic_verification::{EvidenceKind, EvidenceStatus, SemanticEvidence};
    use crate::source_refs::SourceFacts;
    use lean_dup_index::{ComparisonEvidenceMode, ComparisonEvidencePolicy};
    use lean_dup_index::{DeclarationHandle, HydratedDeclaration};
    use lean_dup_worker::Fingerprints;

    #[test]
    fn unverified_mathlib_static_match_is_not_visible_action() {
        let workspace = declaration("workspace:Tiny:Tiny.same", "workspace", "Tiny.same");
        let mathlib = declaration("mathlib:Mathlib:Mathlib.same", "mathlib", "Mathlib.same");
        let review = rank_candidates(input_requiring_mathlib_evidence(vec![candidate_set(
            workspace,
            candidate(mathlib, "statement-fingerprint", 100.0),
        )]));

        let group = &review.groups[0];
        assert_eq!(group.review_priority, ReviewPriority::Noise);
        assert!(group.blockers.contains(&"unverified-proof-grade-evidence".to_owned()));
        assert_ne!(group.recommended_action, ReviewAction::AlreadyInMathlib);
    }

    #[test]
    fn static_mathlib_index_is_actionable_but_not_proof_grade() {
        let workspace = declaration("workspace:Tiny:Tiny.same", "workspace", "Tiny.same");
        let mathlib = declaration("mathlib:Mathlib:Mathlib.same", "mathlib", "Mathlib.same");
        let review = rank_candidates(input(vec![candidate_set(
            workspace,
            candidate(mathlib, "statement-fingerprint", 100.0),
        )]));

        let group = &review.groups[0];
        assert_eq!(group.recommended_action, ReviewAction::AlreadyInMathlib);
        assert_eq!(group.evidence_mode, ReviewEvidenceMode::Static);
    }

    #[test]
    fn verified_mathlib_match_is_high_priority_already_in_mathlib() {
        let workspace = declaration("workspace:Tiny:Tiny.same", "workspace", "Tiny.same");
        let mathlib = declaration("mathlib:Mathlib:Mathlib.same", "mathlib", "Mathlib.same");
        let pair_id = format!("{}::{}", workspace.declaration_id, mathlib.declaration_id);
        let mut candidate = candidate(mathlib, "statement-fingerprint", 100.0);
        candidate.pair_id = pair_id.clone();
        let mut evidence = BTreeMap::new();
        evidence.insert(
            pair_id,
            SemanticEvidence {
                pair_id: "workspace:Tiny:Tiny.same::mathlib:Mathlib:Mathlib.same".to_owned(),
                kind: EvidenceKind::ExactTheorem,
                status: EvidenceStatus::Verified,
                obligation: crate::SearchSemanticObligationKind::ExactTheorem,
                unavailable_reason: None,
                summary: None,
            },
        );
        let review = rank_candidates(RankingInput {
            candidate_sets: &[candidate_set(workspace, candidate)],
            semantic_evidence: &evidence,
            source_facts: &SourceFacts::empty(),
            profile: RankingProfile::default(),
            comparison_policy: Box::leak(Box::new(proof_grade_mathlib_policy())),
        });

        let group = &review.groups[0];
        assert_eq!(group.relation, ReviewRelation::ExactStatement);
        assert_eq!(group.review_priority, ReviewPriority::High);
        assert_eq!(group.recommended_action, ReviewAction::AlreadyInMathlib);
    }

    #[test]
    fn non_theorem_statement_fingerprint_is_not_exact_without_probe() {
        let mut workspace = declaration("workspace:Tiny:Tiny.Shape", "workspace", "Tiny.Shape");
        workspace.kind = "inductive".to_owned();
        let mut mathlib = declaration("mathlib:Mathlib:Mathlib.OtherShape", "mathlib", "Mathlib.OtherShape");
        mathlib.kind = "inductive".to_owned();
        let review = rank_candidates(input(vec![candidate_set(
            workspace,
            candidate(mathlib, "statement-fingerprint", 100.0),
        )]));

        let group = &review.groups[0];
        assert_ne!(group.relation, ReviewRelation::ExactStatement);
        assert_ne!(group.recommended_action, ReviewAction::AlreadyInMathlib);
    }

    #[test]
    fn confirmed_specialization_overrides_general_near_match() {
        let left = declaration("workspace:Tiny:Tiny.general", "workspace", "Tiny.general");
        let right = declaration("workspace:Tiny:Tiny.specific", "workspace", "Tiny.specific");
        let pair_id = "workspace:Tiny:Tiny.general::workspace:Tiny:Tiny.specific".to_owned();
        let mut evidence = BTreeMap::new();
        evidence.insert(
            pair_id.clone(),
            SemanticEvidence {
                pair_id: pair_id.clone(),
                kind: EvidenceKind::Specialization,
                status: EvidenceStatus::Verified,
                obligation: crate::SearchSemanticObligationKind::Specialization,
                unavailable_reason: None,
                summary: None,
            },
        );
        let mut ranked_candidate = candidate(right, "conclusion-fingerprint", 45.0);
        ranked_candidate.pair_id = pair_id;
        let review = rank_candidates(RankingInput {
            candidate_sets: &[candidate_set(left, ranked_candidate)],
            semantic_evidence: &evidence,
            source_facts: &SourceFacts::empty(),
            profile: RankingProfile::default(),
            comparison_policy: Box::leak(Box::new(ComparisonEvidencePolicy::default())),
        });

        assert_eq!(review.groups[0].recommended_action, ReviewAction::SpecializationOf);
        assert_eq!(review.groups[0].relation, ReviewRelation::Specialization);
    }

    #[test]
    fn generated_and_broad_head_only_groups_are_hidden_by_default() {
        let mut left = declaration("workspace:Tiny:Tiny.generated", "workspace", "Tiny.generated");
        left.status_flags.push("generated".to_owned());
        left.low_signal_markers.push("broad_head:Eq".to_owned());
        let mut right = declaration("workspace:Tiny:Tiny.other", "workspace", "Tiny.other");
        right.low_signal_markers.push("broad_head:Eq".to_owned());
        let review = rank_candidates(input(vec![candidate_set(
            left,
            candidate_with_display(right, "role-feature", Some("Eq"), 8.0),
        )]));
        let filter = ReviewFilter {
            include_generated: false,
            include_private: false,
            include_diagnostics: false,
            min_priority: ReviewPriority::Low,
        };

        assert!(review.groups[0].blockers.contains(&"generated-declaration".to_owned()));
        assert!(review.groups[0].blockers.contains(&"broad-head-only".to_owned()));
        assert!(review.visible_groups(filter).is_empty());
    }

    #[test]
    fn rejected_or_unavailable_probe_groups_are_hidden_by_default() {
        for status in [EvidenceStatus::Rejected, EvidenceStatus::Unavailable] {
            let left = declaration("workspace:Tiny:Tiny.left", "workspace", "Tiny.left");
            let right = declaration("workspace:Tiny:Tiny.right", "workspace", "Tiny.right");
            let pair_id = "workspace:Tiny:Tiny.left::workspace:Tiny:Tiny.right".to_owned();
            let mut evidence = BTreeMap::new();
            evidence.insert(
                pair_id.clone(),
                SemanticEvidence {
                    pair_id: pair_id.clone(),
                    kind: EvidenceKind::ExactTheorem,
                    status,
                    obligation: crate::SearchSemanticObligationKind::ExactTheorem,
                    unavailable_reason: None,
                    summary: None,
                },
            );
            let mut ranked_candidate = candidate(right, "statement-fingerprint", 100.0);
            ranked_candidate.pair_id = pair_id;
            let review = rank_candidates(RankingInput {
                candidate_sets: &[candidate_set(left, ranked_candidate)],
                semantic_evidence: &evidence,
                source_facts: &SourceFacts::empty(),
                profile: RankingProfile::default(),
                comparison_policy: Box::leak(Box::new(ComparisonEvidencePolicy::default())),
            });
            let filter = ReviewFilter {
                include_generated: false,
                include_private: false,
                include_diagnostics: false,
                min_priority: ReviewPriority::Medium,
            };

            assert!(review.visible_groups(filter).is_empty());
        }
    }

    #[test]
    fn private_private_helper_groups_are_hidden_until_private_findings_are_requested() {
        let mut left = declaration("workspace:Tiny:Tiny.left_aux", "workspace", "Tiny.left_aux");
        left.visibility = "private".to_owned();
        let mut right = declaration("workspace:Tiny:Tiny.right_aux", "workspace", "Tiny.right_aux");
        right.visibility = "private".to_owned();
        let review = rank_candidates(input(vec![candidate_set(
            left,
            candidate(right, "statement-fingerprint", 100.0),
        )]));
        let default_filter = ReviewFilter {
            include_generated: false,
            include_private: false,
            include_diagnostics: false,
            min_priority: ReviewPriority::Medium,
        };
        let private_filter = ReviewFilter {
            include_generated: false,
            include_private: true,
            include_diagnostics: false,
            min_priority: ReviewPriority::Medium,
        };

        assert_eq!(review.groups[0].review_priority, ReviewPriority::High);
        assert!(review.visible_groups(default_filter).is_empty());
        assert_eq!(review.visible_groups(private_filter).len(), 1);
    }

    #[test]
    fn private_visibility_does_not_include_noise_groups() {
        let mut left = declaration("workspace:Tiny:Tiny.left_aux", "workspace", "Tiny.left_aux");
        left.visibility = "private".to_owned();
        left.low_signal_markers.push("broad_head:Eq".to_owned());
        let mut right = declaration("workspace:Tiny:Tiny.right_aux", "workspace", "Tiny.right_aux");
        right.visibility = "private".to_owned();
        right.low_signal_markers.push("broad_head:Eq".to_owned());
        let review = rank_candidates(input(vec![candidate_set(
            left,
            candidate_with_display(right, "role-feature", Some("Eq"), 8.0),
        )]));
        let private_filter = ReviewFilter {
            include_generated: false,
            include_private: true,
            include_diagnostics: false,
            min_priority: ReviewPriority::Medium,
        };
        let diagnostics_filter = ReviewFilter {
            include_generated: true,
            include_private: true,
            include_diagnostics: true,
            min_priority: ReviewPriority::Noise,
        };

        assert!(review.visible_groups(private_filter).is_empty());
        assert_eq!(review.visible_groups(diagnostics_filter).len(), 1);
    }

    #[test]
    fn exact_evidence_suppresses_weaker_same_pair_relations() {
        let review = rank_candidates(input(vec![candidate_set(
            declaration("workspace:Tiny:Tiny.same", "workspace", "Tiny.same"),
            candidate_with_many(
                declaration("workspace:Tiny:Tiny.same2", "workspace", "Tiny.same2"),
                vec!["statement-fingerprint", "connective-fingerprint", "role-feature"],
            ),
        )]));

        assert_eq!(review.groups[0].relation, ReviewRelation::ExactStatement);
        assert!(!review.suppressed.is_empty());
    }

    fn input(candidate_sets: Vec<CandidateSet>) -> RankingInput<'static> {
        RankingInput {
            candidate_sets: Box::leak(candidate_sets.into_boxed_slice()),
            semantic_evidence: Box::leak(Box::new(BTreeMap::new())),
            source_facts: Box::leak(Box::new(SourceFacts::empty())),
            profile: RankingProfile::default(),
            comparison_policy: Box::leak(Box::new(ComparisonEvidencePolicy::default())),
        }
    }

    fn input_requiring_mathlib_evidence(candidate_sets: Vec<CandidateSet>) -> RankingInput<'static> {
        RankingInput {
            candidate_sets: Box::leak(candidate_sets.into_boxed_slice()),
            semantic_evidence: Box::leak(Box::new(BTreeMap::new())),
            source_facts: Box::leak(Box::new(SourceFacts::empty())),
            profile: RankingProfile::default(),
            comparison_policy: Box::leak(Box::new(proof_grade_mathlib_policy())),
        }
    }

    fn proof_grade_mathlib_policy() -> ComparisonEvidencePolicy {
        ComparisonEvidencePolicy::for_origin("mathlib", ComparisonEvidenceMode::ProofGrade)
    }

    fn candidate_set(anchor: HydratedDeclaration, candidate: RetrievedCandidate) -> CandidateSet {
        CandidateSet {
            anchor,
            candidates: vec![candidate],
        }
    }

    fn candidate(declaration: HydratedDeclaration, contribution_kind: &str, score: f64) -> RetrievedCandidate {
        candidate_with_display(declaration, contribution_kind, None, score)
    }

    fn candidate_with_display(
        declaration: HydratedDeclaration,
        contribution_kind: &str,
        display: Option<&str>,
        score: f64,
    ) -> RetrievedCandidate {
        let pair_id = format!("workspace:Tiny:Tiny.same::{}", declaration.declaration_id);
        RetrievedCandidate {
            pair_id,
            declaration,
            score,
            explanation: CandidateExplanation {
                contributions: vec![KeyContribution {
                    kind: contribution_kind.to_owned(),
                    role: Some("conclusion_head".to_owned()),
                    display: display.map(str::to_owned),
                    key: "k".to_owned(),
                    score,
                }],
            },
            source_evidence: Vec::new(),
        }
    }

    fn candidate_with_many(declaration: HydratedDeclaration, contribution_kinds: Vec<&str>) -> RetrievedCandidate {
        let pair_id = format!("workspace:Tiny:Tiny.same::{}", declaration.declaration_id);
        RetrievedCandidate {
            pair_id,
            declaration,
            score: 120.0,
            explanation: CandidateExplanation {
                contributions: contribution_kinds
                    .into_iter()
                    .map(|kind| KeyContribution {
                        kind: kind.to_owned(),
                        role: None,
                        display: None,
                        key: kind.to_owned(),
                        score: 1.0,
                    })
                    .collect(),
            },
            source_evidence: Vec::new(),
        }
    }

    fn declaration(id: &str, origin: &str, name: &str) -> HydratedDeclaration {
        HydratedDeclaration {
            handle: DeclarationHandle::from_fixture_id(id),
            declaration_id: id.to_owned(),
            origin: origin.to_owned(),
            module: name.rsplit_once('.').map(|(module, _)| module).unwrap_or("").to_owned(),
            qualified_name: name.to_owned(),
            display_name: name.rsplit('.').next().unwrap().to_owned(),
            kind: "theorem".to_owned(),
            visibility: "public".to_owned(),
            modifiers: Vec::new(),
            source_span: None,
            statement_text: "theorem".to_owned(),
            docstring_text: None,
            definition_body_summary: None,
            status_flags: Vec::new(),
            feature_version: "features.roles.v1".to_owned(),
            fingerprints: Fingerprints {
                statement: "statement".to_owned(),
                safe_binder_permutation: "permutation".to_owned(),
                connective_shape: "connective".to_owned(),
                conclusion_shape: "conclusion".to_owned(),
            },
            role_features: Vec::new(),
            binder_count: 0,
            low_signal_markers: Vec::new(),
        }
    }
}
