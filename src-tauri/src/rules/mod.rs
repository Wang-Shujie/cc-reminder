pub mod policy;
pub mod resolve;
pub mod template;

pub use policy::{PolicyDecision, PolicyInput, SuppressReason, evaluate_policy, matches_filters};
pub use resolve::{
    ResolvedRule, StoredGlobalRule, StoredRulePatch, default_rule, required_hook_selection,
    resolve_rule, resolve_stored_rule, validate_rule,
};
pub use template::{DEFAULT_TEMPLATE_ZH, TemplateContext, build_template_context, render_document};
