use crate::amount::{Calories, Grams, Macros, Servings};
use crate::{food, log};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::Path;
use std::str::FromStr;

#[derive(Debug, Deserialize)]
pub struct DayLogOps {
    pub ops: Vec<DayLogOp>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum DayLogOp {
    AddFood {
        name: String,
        servings: Servings,
    },
    AddAdhoc {
        title: String,
        servings: Servings,
        calories: Calories,
        protein_g: Grams,
        fiber_g: Grams,
        fat_g: Grams,
        carbs_g: Grams,
        alcohol_g: Grams,
    },
    Remove {
        row: u32,
    },
    Replace {
        row: u32,
        name: Option<String>,
        servings: Option<Servings>,
        title: Option<String>,
        calories: Option<Calories>,
        protein_g: Option<Grams>,
        fiber_g: Option<Grams>,
        fat_g: Option<Grams>,
        carbs_g: Option<Grams>,
        alcohol_g: Option<Grams>,
    },
}

fn valid_names(foods_dir: &Path) -> String {
    match food::list_food_names(foods_dir) {
        Ok(names) => {
            let shown = names
                .iter()
                .take(10)
                .cloned()
                .collect::<Vec<_>>()
                .join(", ");
            if names.len() > 10 {
                format!("{shown}, … ({} total)", names.len())
            } else {
                shown
            }
        }
        Err(_) => "none available".to_string(),
    }
}

fn food_entry(foods_dir: &Path, name: &str, servings: Servings) -> Result<log::LogEntry, String> {
    let food_name = food::FoodName::from_str(name).map_err(|_| {
        format!(
            "unknown food '{name}' — valid names: {}",
            valid_names(foods_dir)
        )
    })?;
    let path = food_name.file_path(foods_dir);
    if !path.exists() {
        return Err(format!(
            "unknown food '{name}' — valid names: {}",
            valid_names(foods_dir)
        ));
    }
    let f = food::load_food(&path).map_err(|e| format!("failed to load food '{name}': {e}"))?;
    let ps = f.per_serving().map_err(|e| format!("food '{name}': {e}"))?;
    Ok(log::LogEntry {
        title: f.title.clone(),
        servings,
        calories: ps.calories,
        protein_g: ps.protein_g,
        fiber_g: ps.fiber_g,
        fat_g: ps.fat_g,
        carbs_g: ps.carbs_g,
        alcohol_g: ps.alcohol_g,
        timestamp: None,
    })
}

fn replace_entry(foods_dir: &Path, op: &DayLogOp) -> Result<log::LogEntry, String> {
    let DayLogOp::Replace {
        name,
        servings,
        title,
        calories,
        protein_g,
        fiber_g,
        fat_g,
        carbs_g,
        alcohol_g,
        ..
    } = op
    else {
        unreachable!("replace_entry only handles Replace ops")
    };
    match (name, title) {
        (Some(n), None) => {
            let Some(servings) = servings else {
                return Err("replace-food requires `servings`".to_string());
            };
            if calories.is_some()
                || protein_g.is_some()
                || fiber_g.is_some()
                || fat_g.is_some()
                || carbs_g.is_some()
                || alcohol_g.is_some()
            {
                return Err(
                    "replace-food must not include macros — intake computes them from the food file"
                        .to_string(),
                );
            }
            food_entry(foods_dir, n, *servings)
        }
        (None, Some(t)) => {
            let Some(servings) = servings else {
                return Err("replace-adhoc requires `servings`".to_string());
            };
            let Some(calories) = calories else {
                return Err("replace-adhoc requires all six macros".to_string());
            };
            let Some(protein_g) = protein_g else {
                return Err("replace-adhoc requires all six macros".to_string());
            };
            let Some(fiber_g) = fiber_g else {
                return Err("replace-adhoc requires all six macros".to_string());
            };
            let Some(fat_g) = fat_g else {
                return Err("replace-adhoc requires all six macros".to_string());
            };
            let Some(carbs_g) = carbs_g else {
                return Err("replace-adhoc requires all six macros".to_string());
            };
            let Some(alcohol_g) = alcohol_g else {
                return Err("replace-adhoc requires all six macros".to_string());
            };
            Ok(log::LogEntry {
                title: t.clone(),
                servings: *servings,
                calories: *calories,
                protein_g: *protein_g,
                fiber_g: *fiber_g,
                fat_g: *fat_g,
                carbs_g: *carbs_g,
                alcohol_g: *alcohol_g,
                timestamp: None,
            })
        }
        (Some(_), Some(_)) => {
            Err("replace must not combine a food `name` with an ad-hoc `title`".to_string())
        }
        (None, None) => {
            Err("replace requires either a food `name` or an ad-hoc `title`".to_string())
        }
    }
}

pub fn apply_ops(
    day: &log::DayLog,
    ops: &[DayLogOp],
    foods_dir: &Path,
) -> Result<log::DayLog, String> {
    let mut additions: Vec<log::LogEntry> = Vec::new();
    let mut row_ops: BTreeMap<u32, &DayLogOp> = BTreeMap::new();

    for (i, op) in ops.iter().enumerate() {
        let label = format!("op {}", i + 1);
        match op {
            DayLogOp::AddFood { name, servings } => {
                additions.push(
                    food_entry(foods_dir, name, *servings).map_err(|e| format!("{label}: {e}"))?,
                );
            }
            DayLogOp::AddAdhoc {
                title,
                servings,
                calories,
                protein_g,
                fiber_g,
                fat_g,
                carbs_g,
                alcohol_g,
            } => {
                additions.push(log::LogEntry {
                    title: title.clone(),
                    servings: *servings,
                    calories: *calories,
                    protein_g: *protein_g,
                    fiber_g: *fiber_g,
                    fat_g: *fat_g,
                    carbs_g: *carbs_g,
                    alcohol_g: *alcohol_g,
                    timestamp: None,
                });
            }
            DayLogOp::Remove { row } | DayLogOp::Replace { row, .. } => {
                let r = *row;
                if r == 0 || r as usize > day.entries.len() {
                    return Err(format!(
                        "{label}: row {r} is out of range — the day has {} (rows 1..={})",
                        log::entry_count_label(day.entries.len()),
                        day.entries.len()
                    ));
                }
                if row_ops.insert(r, op).is_some() {
                    return Err(format!(
                        "{label}: row {r} is already targeted by another op — remove/replace ops on the same row conflict"
                    ));
                }
            }
        }
    }

    let mut entries: Vec<log::LogEntry> = Vec::new();
    for (i, entry) in day.entries.iter().enumerate() {
        let row = (i + 1) as u32;
        match row_ops.remove(&row) {
            None => entries.push(entry.clone()),
            Some(DayLogOp::Remove { .. }) => {}
            Some(op @ DayLogOp::Replace { .. }) => {
                let mut replaced =
                    replace_entry(foods_dir, op).map_err(|e| format!("row {row}: {e}"))?;
                // A replace edits an existing entry's content in place; it
                // keeps the row's original timestamp rather than re-logging.
                replaced.timestamp = entry.timestamp;
                entries.push(replaced);
            }
            Some(_) => unreachable!(),
        }
    }
    entries.extend(additions);

    let mut day_totals = Macros::ZERO;
    for (i, entry) in entries.iter().enumerate() {
        let totals = entry.totals().map_err(|_| {
            format!(
                "entry {} ({}): per-serving macros × servings overflow — use a smaller `servings`, or an `add-adhoc` op with explicit macros",
                i + 1,
                entry.title
            )
        })?;
        day_totals = day_totals.checked_add(&totals).ok_or_else(|| {
            "the day's macro totals overflow — reduce the `servings` or portion sizes".to_string()
        })?;
    }

    Ok(log::DayLog {
        entries,
        exercise_calories: day.exercise_calories,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    fn grams(value: &str) -> Grams {
        Grams::from_str(value).unwrap()
    }

    fn entry(title: &str, servings: &str, calories: &str) -> log::LogEntry {
        log::LogEntry {
            title: title.to_string(),
            servings: Servings::from_str(servings).unwrap(),
            calories: Calories::from_str(calories).unwrap(),
            protein_g: grams("0"),
            fiber_g: grams("0"),
            fat_g: grams("0"),
            carbs_g: grams("0"),
            alcohol_g: grams("0"),
            timestamp: None,
        }
    }

    fn day(entries: Vec<log::LogEntry>) -> log::DayLog {
        log::DayLog {
            entries,
            exercise_calories: Calories::from_str("300").unwrap(),
        }
    }

    fn parse_ops(s: &str) -> Vec<DayLogOp> {
        let ops: DayLogOps = toml::from_str(s).unwrap();
        ops.ops
    }

    fn foods_dir_with_oatmeal() -> tempfile::TempDir {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(
            dir.path().join("oatmeal.toml"),
            "title = \"Oatmeal\"\nservings = 1\n\n[[ingredients]]\nname = \"Oats\"\nquantity = \"100g\"\ncalories = 200\nprotein_g = 10\nfiber_g = 5\nfat_g = 4\ncarbs_g = 30\nalcohol_g = 0\n",
        )
        .unwrap();
        dir
    }

    fn assert_entry_matches(e: &log::LogEntry, title: &str, servings: &str, calories: &str) {
        assert_eq!(e.title, title);
        assert_eq!(e.servings, Servings::from_str(servings).unwrap());
        assert_eq!(e.calories, Calories::from_str(calories).unwrap());
    }

    #[test]
    fn test_add_food_appends_and_computes_macros() {
        let dir = foods_dir_with_oatmeal();
        let day = day(vec![entry("coffee", "1", "12")]);
        let ops = parse_ops("[[ops]]\nkind = \"add-food\"\nname = \"oatmeal\"\nservings = 2\n");
        let applied = apply_ops(&day, &ops, dir.path()).unwrap();
        assert_eq!(applied.entries.len(), 2);
        assert_entry_matches(&applied.entries[0], "coffee", "1", "12");
        assert_entry_matches(&applied.entries[1], "Oatmeal", "2", "200");
        assert_eq!(
            applied.exercise_calories,
            Calories::from_str("300").unwrap()
        );
    }

    #[test]
    fn test_add_food_fractional_servings() {
        let dir = foods_dir_with_oatmeal();
        let day = day(vec![]);
        let ops = parse_ops("[[ops]]\nkind = \"add-food\"\nname = \"oatmeal\"\nservings = 1.5\n");
        let applied = apply_ops(&day, &ops, dir.path()).unwrap();
        assert_eq!(applied.entries.len(), 1);
        assert_eq!(
            applied.entries[0].servings,
            Servings::from_str("1.5").unwrap()
        );
    }

    #[test]
    fn test_add_food_unknown_name_errors_with_hint() {
        let dir = foods_dir_with_oatmeal();
        let day = day(vec![]);
        let ops = parse_ops("[[ops]]\nkind = \"add-food\"\nname = \"ghost\"\nservings = 1\n");
        let err = apply_ops(&day, &ops, dir.path()).unwrap_err();
        assert!(err.contains("unknown food 'ghost'"));
        assert!(err.contains("oatmeal"));
    }

    #[test]
    fn test_add_food_invalid_name_errors() {
        let dir = foods_dir_with_oatmeal();
        let day = day(vec![]);
        let ops = parse_ops("[[ops]]\nkind = \"add-food\"\nname = \"a/b\"\nservings = 1\n");
        let err = apply_ops(&day, &ops, dir.path()).unwrap_err();
        assert!(err.contains("unknown food 'a/b'"));
    }

    #[test]
    fn test_add_adhoc_appends_verbatim_macros() {
        let dir = foods_dir_with_oatmeal();
        let day = day(vec![]);
        let ops = parse_ops(
            "[[ops]]\nkind = \"add-adhoc\"\ntitle = \"Almonds - 30g\"\nservings = 1\ncalories = 164\nprotein_g = 6\nfiber_g = 3.5\nfat_g = 14\ncarbs_g = 6\nalcohol_g = 0\n",
        );
        let applied = apply_ops(&day, &ops, dir.path()).unwrap();
        assert_eq!(applied.entries.len(), 1);
        let e = &applied.entries[0];
        assert_eq!(e.title, "Almonds - 30g");
        assert_eq!(e.fiber_g, grams("3.5"));
        assert_eq!(e.calories, Calories::from_str("164").unwrap());
    }

    #[test]
    fn test_add_adhoc_missing_macro_rejected_at_parse() {
        let result: Result<DayLogOps, _> = toml::from_str(
            "[[ops]]\nkind = \"add-adhoc\"\ntitle = \"X\"\nservings = 1\ncalories = 10\nprotein_g = 1\nfiber_g = 0\nfat_g = 0\ncarbs_g = 0\n",
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_remove_drops_row() {
        let dir = foods_dir_with_oatmeal();
        let day = day(vec![
            entry("coffee", "1", "12"),
            entry("oatmeal", "1", "200"),
            entry("chili", "1", "300"),
        ]);
        let ops = parse_ops("[[ops]]\nkind = \"remove\"\nrow = 2\n");
        let applied = apply_ops(&day, &ops, dir.path()).unwrap();
        assert_eq!(applied.entries.len(), 2);
        assert_eq!(applied.entries[0].title, "coffee");
        assert_eq!(applied.entries[1].title, "chili");
    }

    #[test]
    fn test_remove_row_out_of_range_errors() {
        let dir = foods_dir_with_oatmeal();
        let day = day(vec![entry("coffee", "1", "12")]);
        let ops = parse_ops("[[ops]]\nkind = \"remove\"\nrow = 3\n");
        let err = apply_ops(&day, &ops, dir.path()).unwrap_err();
        assert!(err.contains("row 3 is out of range"));
        assert!(err.contains("rows 1..=1"));
    }

    #[test]
    fn test_remove_row_zero_errors() {
        let dir = foods_dir_with_oatmeal();
        let day = day(vec![entry("coffee", "1", "12")]);
        let ops = parse_ops("[[ops]]\nkind = \"remove\"\nrow = 0\n");
        assert!(apply_ops(&day, &ops, dir.path()).is_err());
    }

    #[test]
    fn test_replace_food_keeps_position() {
        let dir = foods_dir_with_oatmeal();
        let day = day(vec![
            entry("coffee", "1", "12"),
            entry("chili", "1", "300"),
            entry("oatmeal", "1", "200"),
        ]);
        let ops =
            parse_ops("[[ops]]\nkind = \"replace\"\nrow = 2\nname = \"oatmeal\"\nservings = 2\n");
        let applied = apply_ops(&day, &ops, dir.path()).unwrap();
        assert_eq!(applied.entries.len(), 3);
        assert_entry_matches(&applied.entries[1], "Oatmeal", "2", "200");
        assert_eq!(applied.entries[2].title, "oatmeal");
    }

    #[test]
    fn test_replace_adhoc_keeps_position() {
        let dir = foods_dir_with_oatmeal();
        let day = day(vec![entry("coffee", "1", "12"), entry("chili", "1", "300")]);
        let ops = parse_ops(
            "[[ops]]\nkind = \"replace\"\nrow = 2\ntitle = \"Shake\"\nservings = 1\ncalories = 400\nprotein_g = 30\nfiber_g = 2\nfat_g = 10\ncarbs_g = 40\nalcohol_g = 0\n",
        );
        let applied = apply_ops(&day, &ops, dir.path()).unwrap();
        assert_eq!(applied.entries.len(), 2);
        assert_eq!(applied.entries[1].title, "Shake");
        assert_eq!(applied.entries[1].protein_g, grams("30"));
    }

    #[test]
    fn test_replace_keeps_original_timestamp() {
        let dir = foods_dir_with_oatmeal();
        let date = chrono::NaiveDate::from_ymd_opt(2026, 8, 11).unwrap();
        let ts =
            log::Timestamp::from_local(date, chrono::NaiveTime::from_hms_opt(12, 30, 0).unwrap())
                .unwrap();
        let mut original = entry("coffee", "1", "12");
        original.timestamp = Some(ts);
        let day = day(vec![original]);
        let ops =
            parse_ops("[[ops]]\nkind = \"replace\"\nrow = 1\nname = \"oatmeal\"\nservings = 1\n");
        let applied = apply_ops(&day, &ops, dir.path()).unwrap();
        assert_entry_matches(&applied.entries[0], "Oatmeal", "1", "200");
        assert_eq!(
            applied.entries[0].timestamp,
            Some(ts),
            "a replace edits content in place and must keep the row's timestamp"
        );
    }

    #[test]
    fn test_replace_keeps_missing_timestamp_missing() {
        let dir = foods_dir_with_oatmeal();
        let day = day(vec![entry("coffee", "1", "12")]);
        let ops =
            parse_ops("[[ops]]\nkind = \"replace\"\nrow = 1\nname = \"oatmeal\"\nservings = 1\n");
        let applied = apply_ops(&day, &ops, dir.path()).unwrap();
        assert_eq!(applied.entries[0].timestamp, None);
    }

    #[test]
    fn test_replace_food_with_macros_rejected() {
        let dir = foods_dir_with_oatmeal();
        let day = day(vec![entry("coffee", "1", "12")]);
        let ops = parse_ops(
            "[[ops]]\nkind = \"replace\"\nrow = 1\nname = \"oatmeal\"\nservings = 1\ncalories = 5\n",
        );
        let err = apply_ops(&day, &ops, dir.path()).unwrap_err();
        assert!(err.contains("must not include macros"));
    }

    #[test]
    fn test_replace_missing_target_rejected() {
        let dir = foods_dir_with_oatmeal();
        let day = day(vec![entry("coffee", "1", "12")]);
        let ops = parse_ops("[[ops]]\nkind = \"replace\"\nrow = 1\nservings = 1\n");
        let err = apply_ops(&day, &ops, dir.path()).unwrap_err();
        assert!(err.contains("requires either a food"));
    }

    #[test]
    fn test_replace_both_name_and_title_rejected() {
        let dir = foods_dir_with_oatmeal();
        let day = day(vec![entry("coffee", "1", "12")]);
        let ops = parse_ops(
            "[[ops]]\nkind = \"replace\"\nrow = 1\nname = \"oatmeal\"\ntitle = \"X\"\nservings = 1\n",
        );
        let err = apply_ops(&day, &ops, dir.path()).unwrap_err();
        assert!(err.contains("must not combine"));
    }

    #[test]
    fn test_replace_adhoc_missing_macros_rejected() {
        let dir = foods_dir_with_oatmeal();
        let day = day(vec![entry("coffee", "1", "12")]);
        let ops = parse_ops(
            "[[ops]]\nkind = \"replace\"\nrow = 1\ntitle = \"X\"\nservings = 1\ncalories = 10\n",
        );
        let err = apply_ops(&day, &ops, dir.path()).unwrap_err();
        assert!(err.contains("requires all six macros"));
    }

    #[test]
    fn test_duplicate_ops_on_same_row_conflict() {
        let dir = foods_dir_with_oatmeal();
        let day = day(vec![entry("coffee", "1", "12"), entry("chili", "1", "300")]);
        let ops = parse_ops(
            "[[ops]]\nkind = \"remove\"\nrow = 1\n\n[[ops]]\nkind = \"replace\"\nrow = 1\nname = \"oatmeal\"\nservings = 1\n",
        );
        let err = apply_ops(&day, &ops, dir.path()).unwrap_err();
        assert!(err.contains("conflict"));
        assert!(err.contains("row 1"));
    }

    #[test]
    fn test_two_removes_on_same_row_conflict() {
        let dir = foods_dir_with_oatmeal();
        let day = day(vec![entry("coffee", "1", "12")]);
        let ops = parse_ops(
            "[[ops]]\nkind = \"remove\"\nrow = 1\n\n[[ops]]\nkind = \"remove\"\nrow = 1\n",
        );
        let err = apply_ops(&day, &ops, dir.path()).unwrap_err();
        assert!(err.contains("conflict"));
    }

    #[test]
    fn test_empty_ops_returns_same_day() {
        let dir = foods_dir_with_oatmeal();
        let day = day(vec![entry("coffee", "1", "12")]);
        let applied = apply_ops(&day, &[], dir.path()).unwrap();
        assert_eq!(applied, day);
    }

    #[test]
    fn test_remove_all_entries_empties_day_but_keeps_exercise() {
        let dir = foods_dir_with_oatmeal();
        let day = day(vec![entry("coffee", "1", "12")]);
        let ops = parse_ops("[[ops]]\nkind = \"remove\"\nrow = 1\n");
        let applied = apply_ops(&day, &ops, dir.path()).unwrap();
        assert!(applied.entries.is_empty());
        assert_eq!(
            applied.exercise_calories,
            Calories::from_str("300").unwrap()
        );
    }

    #[test]
    fn test_huge_servings_overflow_is_retryable_error() {
        let dir = foods_dir_with_oatmeal();
        let day = day(vec![entry("coffee", "1", "12")]);
        let ops = parse_ops("[[ops]]\nkind = \"add-food\"\nname = \"oatmeal\"\nservings = 1e28\n");
        let err = apply_ops(&day, &ops, dir.path()).unwrap_err();
        assert!(err.contains("overflow"), "got: {err}");
        assert!(err.contains("smaller `servings`"), "got: {err}");
    }

    #[test]
    fn test_day_totals_overflow_is_retryable_error() {
        let dir = foods_dir_with_oatmeal();
        let day = day(vec![]);
        let max = Calories::from_decimal(rust_decimal::Decimal::MAX).unwrap();
        let op = |title: &str| DayLogOp::AddAdhoc {
            title: title.to_string(),
            servings: Servings::ONE,
            calories: max,
            protein_g: grams("0"),
            fiber_g: grams("0"),
            fat_g: grams("0"),
            carbs_g: grams("0"),
            alcohol_g: grams("0"),
        };
        let ops = vec![op("A"), op("B")];
        let err = apply_ops(&day, &ops, dir.path()).unwrap_err();
        assert!(err.contains("day's macro totals overflow"), "got: {err}");
    }

    #[test]
    fn test_servings_literal_beyond_decimal_rejected_at_parse() {
        let result: Result<DayLogOps, _> = toml::from_str(
            "[[ops]]\nkind = \"add-food\"\nname = \"x\"\nservings = 99999999999999999999999999\n",
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_unknown_kind_rejected_at_parse() {
        assert!(toml::from_str::<DayLogOps>("[[ops]]\nkind = \"bogus\"\n").is_err());
    }

    #[test]
    fn test_quoted_servings_rejected() {
        assert!(toml::from_str::<DayLogOps>(
            "[[ops]]\nkind = \"add-food\"\nname = \"x\"\nservings = \"1\"\n"
        )
        .is_err());
    }

    #[test]
    fn test_apply_ops_ignores_notes_about_order_of_ops_on_different_rows() {
        let dir = foods_dir_with_oatmeal();
        let day = day(vec![
            entry("coffee", "1", "12"),
            entry("chili", "1", "300"),
            entry("shake", "1", "400"),
        ]);
        let ops = parse_ops(
            "[[ops]]\nkind = \"remove\"\nrow = 1\n\n[[ops]]\nkind = \"replace\"\nrow = 3\nname = \"oatmeal\"\nservings = 1\n",
        );
        let applied = apply_ops(&day, &ops, dir.path()).unwrap();
        assert_eq!(applied.entries.len(), 2);
        assert_eq!(applied.entries[0].title, "chili");
        assert_eq!(applied.entries[1].title, "Oatmeal");
    }
}
