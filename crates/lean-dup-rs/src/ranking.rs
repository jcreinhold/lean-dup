use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::index::HydratedDeclaration;
use crate::retrieval::{CandidateSet, KeyContribution, RetrievedCandidate};
use crate::source_refs::{ImportStatus, SourceFacts};
use crate::worker::{ProbeResult, SourceSpan};

const DEFAULT_MIN_NEAR_SCORE: f64 = 24.0;
const DEFAULT_TRANSITIONAL_ALIAS_CALLERS: usize = 8;

/// Policy input for turning retrieved candidates into review groups.
///
/// Ranking consumes typed semantic and source facts. It does not query storage,
/// run probes, parse worker transport records, parse CLI flags, or render text.
#[derive(Debug, Clone)]
pub(crate) struct RankingInput<'a> {
    pub(crate) candidate_sets: &'a [CandidateSet],
    pub(crate) probe_results: &'a BTreeMap<String, ProbeResult>,
    pub(crate) source_facts: &'a SourceFacts,
    pub(crate) profile: RankingProfile,
}

/// Tunable ranking defaults owned by a review profile, not by CLI parsing.
#[derive(Debug, Clone, Copy)]
pub(crate) struct RankingProfile {
    pub(crate) min_near_score: f64,
    pub(crate) transitional_alias_callers: usize,
}

impl Default for RankingProfile {
    fn default() -> Self {
        Self {
            min_near_score: DEFAULT_MIN_NEAR_SCORE,
            transitional_alias_callers: DEFAULT_TRANSITIONAL_ALIAS_CALLERS,
        }
    }
}

/// Ranked review queue plus diagnostics that later renderers can present.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct RankedReview {
    pub(crate) groups: Vec<RankedGroup>,
    pub(crate) suppressed: Vec<SuppressedGroup>,
    pub(crate) diagnostics: RankingDiagnostics,
}

impl RankedReview {
    pub(crate) fn visible_groups(&self, filter: ReviewFilter) -> Vec<&RankedGroup> {
        self.groups.iter().filter(|group| filter.includes(group)).collect()
    }
}

/// User-facing filter policy for default queues.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ReviewFilter {
    pub(crate) include_generated: bool,
    pub(crate) show_noise: bool,
    pub(crate) min_priority: ReviewPriority,
}

impl ReviewFilter {
    pub(crate) fn includes(self, group: &RankedGroup) -> bool {
        if group.review_priority == ReviewPriority::Noise && !self.show_noise {
            return false;
        }
        if !self.include_generated && group.blockers.iter().any(|blocker| blocker == "generated-declaration") {
            return false;
        }
        group.review_priority <= self.min_priority
    }
}

/// One ranked candidate group ready for review or rendering.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct RankedGroup {
    pub(crate) id: String,
    pub(crate) pair_id: String,
    pub(crate) relation: ReviewRelation,
    pub(crate) members: Vec<ReviewMember>,
    pub(crate) evidence: Vec<ReviewEvidence>,
    pub(crate) signals: Vec<String>,
    pub(crate) blockers: Vec<String>,
    pub(crate) confidence: ConfidenceTier,
    pub(crate) review_priority: ReviewPriority,
    pub(crate) recommended_action: ReviewAction,
    pub(crate) target_decl: Option<String>,
    pub(crate) target_module: Option<String>,
    pub(crate) probe_summary: Option<String>,
    pub(crate) local_caller_count: usize,
    pub(crate) replacement_hint: Option<crate::replacement_hints::ReplacementHint>,
}

/// Declaration facts exposed to review output.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct ReviewMember {
    pub(crate) declaration_id: String,
    pub(crate) origin: String,
    pub(crate) module: String,
    pub(crate) qualified_name: String,
    pub(crate) display_name: String,
    pub(crate) kind: String,
    pub(crate) visibility: String,
    pub(crate) source_span: Option<SourceSpan>,
    pub(crate) status_flags: Vec<String>,
}

/// One typed evidence item supporting a ranked group.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct ReviewEvidence {
    pub(crate) kind: String,
    pub(crate) role: Option<String>,
    pub(crate) display: Option<String>,
    pub(crate) score: f64,
}

impl ReviewEvidence {
    pub(crate) fn summary(&self) -> String {
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
pub(crate) enum ReviewRelation {
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
pub(crate) enum ReviewAction {
    AlreadyInMathlib,
    LocalAlias,
    ReplaceLocalUses,
    MergeGeneralization,
    SpecializationOf,
    ProbableSourceClone,
    ManualReview,
}

/// Confidence tier for queue ordering and default filtering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum ConfidenceTier {
    High,
    Medium,
    Low,
    Noise,
}

/// Review priority. Lower variants sort earlier and are included by stricter filters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum ReviewPriority {
    High,
    Medium,
    Low,
    Noise,
}

/// A weaker relation hidden because a stronger relation covers the same pair.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct SuppressedGroup {
    pub(crate) pair_id: String,
    pub(crate) suppressed_relation: ReviewRelation,
    pub(crate) covered_by: ReviewRelation,
    pub(crate) reason: String,
}

/// Ranking counters for diagnostics and later `show` support.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub(crate) struct RankingDiagnostics {
    pub(crate) candidate_pairs: usize,
    pub(crate) emitted_groups: usize,
    pub(crate) suppressed_groups: usize,
}

/// Rank retrieved candidates into actionable review groups.
pub(crate) fn rank_candidates(input: RankingInput<'_>) -> RankedReview {
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
    let probe = input.probe_results.get(&candidate.pair_id);
    let mut signals = contribution_signals(&candidate.explanation.contributions);
    let mut blockers = BTreeSet::new();
    let source_clone = same_source_fingerprint(anchor, &candidate.declaration, input.source_facts);
    let has_mathlib = candidate.declaration.origin == "mathlib" || anchor.origin == "mathlib";
    let exact = has_contribution(candidate, "statement-fingerprint")
        || probe.is_some_and(|probe| probe.same_statement || probe.same_reducible_definition);
    let specialization = probe.is_some_and(|probe| probe.specializes_left_to_right || probe.specializes_right_to_left);
    let permuted = has_contribution(candidate, "safe-permutation-fingerprint")
        || probe.is_some_and(|probe| probe.same_up_to_safe_reordering);
    let connective =
        has_contribution(candidate, "connective-fingerprint") || probe.is_some_and(|probe| probe.connective_equivalent);
    let near = candidate.score >= input.profile.min_near_score || has_contribution(candidate, "conclusion-fingerprint");

    if let Some(probe) = probe {
        signals.extend(probe_signals(probe));
        if probe.status != "ok" {
            blockers.insert("lean-probe-unavailable".to_owned());
        }
    }
    if source_clone {
        signals.insert("source-clone".to_owned());
    }
    if is_generated(anchor) || is_generated(&candidate.declaration) {
        blockers.insert("generated-declaration".to_owned());
    }
    if broad_head_only(anchor, &candidate.declaration, &candidate.explanation.contributions) {
        blockers.insert("broad-head-only".to_owned());
    }
    if typeclass_instance_noise(anchor) || typeclass_instance_noise(&candidate.declaration) {
        blockers.insert("typeclass-instance-noise".to_owned());
    }
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
    if blockers.contains("generated-declaration") || blockers.contains("broad-head-only") {
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
        probe_summary: probe.and_then(probe_summary),
        local_caller_count,
        replacement_hint: None,
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

fn probe_signals(probe: &ProbeResult) -> BTreeSet<String> {
    let mut signals = BTreeSet::new();
    if probe.same_statement {
        signals.insert("probe:same-statement".to_owned());
    }
    if probe.same_up_to_safe_reordering {
        signals.insert("probe:same-up-to-safe-reordering".to_owned());
    }
    if probe.connective_equivalent {
        signals.insert("probe:connective-equivalent".to_owned());
    }
    if probe.specializes_left_to_right {
        signals.insert("probe:specializes-left-to-right".to_owned());
    }
    if probe.specializes_right_to_left {
        signals.insert("probe:specializes-right-to-left".to_owned());
    }
    if probe.same_reducible_definition {
        signals.insert("probe:same-reducible-definition".to_owned());
    }
    signals
}

fn has_contribution(candidate: &RetrievedCandidate, kind: &str) -> bool {
    candidate
        .explanation
        .contributions
        .iter()
        .any(|contribution| contribution.kind == kind)
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

fn is_generated(declaration: &HydratedDeclaration) -> bool {
    declaration.status_flags.iter().any(|flag| flag == "generated")
}

fn typeclass_instance_noise(declaration: &HydratedDeclaration) -> bool {
    declaration.kind == "instance" || declaration.display_name.starts_with("inst")
}

fn broad_head_only(left: &HydratedDeclaration, right: &HydratedDeclaration, contributions: &[KeyContribution]) -> bool {
    if contributions.is_empty() {
        return false;
    }
    let broad_heads = left
        .low_signal_markers
        .iter()
        .chain(right.low_signal_markers.iter())
        .filter_map(|marker| marker.strip_prefix("broad_head:"))
        .collect::<BTreeSet<_>>();
    !broad_heads.is_empty()
        && contributions.iter().all(|contribution| {
            contribution.kind == "role-feature"
                && contribution
                    .display
                    .as_deref()
                    .is_some_and(|display| broad_heads.contains(display))
        })
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

fn probe_summary(probe: &ProbeResult) -> Option<String> {
    probe.message.clone().or_else(|| {
        if probe.status == "ok" {
            None
        } else {
            Some(format!("probe status {}", probe.status))
        }
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
        RankingInput, RankingProfile, ReviewAction, ReviewFilter, ReviewPriority, ReviewRelation, rank_candidates,
    };
    use crate::index::{DeclarationHandle, HydratedDeclaration};
    use crate::retrieval::{CandidateExplanation, CandidateSet, KeyContribution, RetrievedCandidate};
    use crate::source_refs::SourceFacts;
    use crate::worker::{Fingerprints, ProbeResult};

    #[test]
    fn exact_mathlib_match_is_high_priority_already_in_mathlib() {
        let workspace = declaration("workspace:Tiny:Tiny.same", "workspace", "Tiny.same");
        let mathlib = declaration("mathlib:Mathlib:Mathlib.same", "mathlib", "Mathlib.same");
        let review = rank_candidates(input(vec![candidate_set(
            workspace,
            candidate(mathlib, "statement-fingerprint", 100.0),
        )]));

        let group = &review.groups[0];
        assert_eq!(group.review_priority, ReviewPriority::High);
        assert_eq!(group.recommended_action, ReviewAction::AlreadyInMathlib);
        assert_eq!(group.relation, ReviewRelation::ExactStatement);
    }

    #[test]
    fn confirmed_specialization_overrides_general_near_match() {
        let left = declaration("workspace:Tiny:Tiny.general", "workspace", "Tiny.general");
        let right = declaration("workspace:Tiny:Tiny.specific", "workspace", "Tiny.specific");
        let pair_id = "workspace:Tiny:Tiny.general::workspace:Tiny:Tiny.specific".to_owned();
        let mut probes = BTreeMap::new();
        probes.insert(
            pair_id.clone(),
            ProbeResult {
                pair_id: pair_id.clone(),
                left_declaration_id: left.declaration_id.clone(),
                right_declaration_id: right.declaration_id.clone(),
                status: "ok".to_owned(),
                same_statement: false,
                same_up_to_safe_reordering: false,
                connective_equivalent: false,
                specializes_left_to_right: true,
                specializes_right_to_left: false,
                mutual_implication_shape: false,
                same_reducible_definition: false,
                message: None,
            },
        );
        let mut ranked_candidate = candidate(right, "conclusion-fingerprint", 45.0);
        ranked_candidate.pair_id = pair_id;
        let review = rank_candidates(RankingInput {
            candidate_sets: &[candidate_set(left, ranked_candidate)],
            probe_results: &probes,
            source_facts: &SourceFacts::empty(),
            profile: RankingProfile::default(),
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
            show_noise: false,
            min_priority: ReviewPriority::Low,
        };

        assert!(review.groups[0].blockers.contains(&"generated-declaration".to_owned()));
        assert!(review.groups[0].blockers.contains(&"broad-head-only".to_owned()));
        assert!(review.visible_groups(filter).is_empty());
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
            probe_results: Box::leak(Box::new(BTreeMap::new())),
            source_facts: Box::leak(Box::new(SourceFacts::empty())),
            profile: RankingProfile::default(),
        }
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
        }
    }

    fn declaration(id: &str, origin: &str, name: &str) -> HydratedDeclaration {
        HydratedDeclaration {
            handle: DeclarationHandle::for_test(id),
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
