// All prompts are spliced around query_style.md so the shared usda_search
// rules live once and cannot drift between templates. The food_lookup rules
// are log-only and live in log_head.md.
pub const LOG: &str = concat!(
    include_str!("prompts/log_head.md"),
    include_str!("prompts/query_style.md"),
    include_str!("prompts/log_tail.md"),
);
pub const FOOD_NEW: &str = concat!(
    include_str!("prompts/food_new_head.md"),
    include_str!("prompts/query_style.md"),
    include_str!("prompts/food_new_tail.md"),
);
pub const FOOD_EDIT: &str = concat!(
    include_str!("prompts/food_edit_head.md"),
    include_str!("prompts/query_style.md"),
    include_str!("prompts/food_edit_tail.md"),
);
