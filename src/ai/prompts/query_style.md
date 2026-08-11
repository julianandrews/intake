- Query `usda_search` with the food's identifying words (brand, type, variant —
  e.g. "cabot cheddar" or "cheddar cheese"); never quantities, units, or
  portion words. The result line's name carries the variant (raw vs cooked,
  whole vs skim). Batch queries in one call — round trips are expensive.
- When a tool result exists, scale its per-100g values to the amount yourself
  (amount ÷ 100 × each value), rounding to whole calories and 0.1 g — never
  estimate or recompute from memory when a result exists.
