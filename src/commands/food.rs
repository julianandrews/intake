use crate::config::{Column, Config};
use crate::display;
use crate::display::{colorize, food_cell, Align, ColumnValue, Table};
use crate::food;
use anyhow::{Context, Result};
use std::io::Write;
use std::path::Path;

fn render_food(food: &food::Food, columns: &[Column]) -> Result<String> {
    let serving_label = if food.servings.get() == 1 {
        "serving"
    } else {
        "servings"
    };

    let mut headers: Vec<&str> = vec!["Ingredient", "Amount"];
    let mut aligns = vec![Align::Left, Align::Left];
    for column in columns {
        headers.push(column.label());
        aligns.push(Align::Right);
    }

    let mut table = Table::with_align(&headers, &aligns);
    table.set_title(&format!(
        "{} ({} {})",
        food.title, food.servings, serving_label
    ));

    for ing in &food.ingredients {
        let qty = ing.quantity.as_deref().unwrap_or("-").to_string();
        let mut cells = vec![ing.name.clone(), qty];
        for column in columns {
            cells.push(food_cell(*column, ing.column_value(*column)));
        }
        table.add_row(cells);
    }

    let t = food.totals()?;

    let mut total = vec!["Total".to_string(), String::new()];
    for column in columns {
        total.push(food_cell(*column, t.column_value(*column)));
    }
    table.add_footer(total);

    let ps = food.per_serving()?;
    let mut per_serving = vec!["Per serving".to_string(), String::new()];
    for column in columns {
        per_serving.push(food_cell(*column, ps.column_value(*column)));
    }
    table.add_footer(per_serving);

    let mut out = table.format();
    if !food.notes.trim().is_empty() {
        out.push('\n');
        out.push_str(&colorize("Notes:", display::ANSI_BOLD_YELLOW));
        out.push('\n');
        out.push_str(&colorize(&food.notes, display::ANSI_DIM));
        out.push('\n');
    }

    Ok(out)
}

pub(crate) fn cmd_show_food(
    writer: &mut impl Write,
    foods_dir: &Path,
    slug: &food::Slug,
    config: &Config,
) -> Result<()> {
    let food = food::load_food(&slug.file_path(foods_dir))
        .with_context(|| format!("food '{}' not found", slug))?;
    write!(writer, "{}", render_food(&food, &config.columns()?)?)?;
    Ok(())
}

pub(crate) fn cmd_list(writer: &mut impl Write, foods_dir: &Path, config: &Config) -> Result<()> {
    let foods = food::find_all_foods(foods_dir)?;
    let columns = config.columns()?;

    let mut headers: Vec<&str> = vec!["Food", "Servings"];
    for column in &columns {
        headers.push(if *column == Column::Calories {
            "Cal/serv"
        } else {
            column.label()
        });
    }

    let mut table = Table::new(&headers);
    table.set_title("All Foods");

    for food in &foods {
        let ps = food.per_serving()?;
        let mut cells = vec![food.title.clone(), food.servings.to_string()];
        for column in &columns {
            cells.push(display::food_cell(*column, ps.column_value(*column)));
        }
        table.add_row(cells);
    }

    write!(writer, "{}", table.format())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::amount::{Calories, Grams};
    use crate::config::DEFAULT_COLUMNS;
    use std::num::NonZeroU32;
    use std::str::FromStr;

    fn grams(value: &str) -> Grams {
        Grams::from_str(value).unwrap()
    }

    fn ingredient(
        protein: &str,
        fiber: &str,
        calories: u32,
        fat: &str,
        carbs: &str,
        alcohol: &str,
    ) -> food::Ingredient {
        food::Ingredient {
            name: "A".to_string(),
            quantity: None,
            protein_g: grams(protein),
            fiber_g: grams(fiber),
            calories: Calories::from_str(&calories.to_string()).unwrap(),
            fat_g: grams(fat),
            carbs_g: grams(carbs),
            alcohol_g: grams(alcohol),
        }
    }

    fn food_with_ingredient(servings: u32, ingredient: food::Ingredient) -> food::Food {
        food::Food {
            title: "Test".to_string(),
            servings: NonZeroU32::new(servings).unwrap(),
            notes: String::new(),
            ingredients: vec![ingredient],
        }
    }

    #[test]
    fn test_render_food_basic() {
        let food = food::Food {
            title: "Oatmeal".to_string(),
            servings: NonZeroU32::new(2).unwrap(),
            notes: String::new(),
            ingredients: vec![
                food::Ingredient {
                    name: "Oats".to_string(),
                    quantity: Some("100g".to_string()),
                    protein_g: grams("10.0"),
                    fiber_g: grams("5.0"),
                    calories: Calories::from_str("200").unwrap(),
                    fat_g: grams("4.0"),
                    carbs_g: grams("30.0"),
                    alcohol_g: grams("0.0"),
                },
                food::Ingredient {
                    name: "Milk".to_string(),
                    quantity: Some("200ml".to_string()),
                    protein_g: grams("8.0"),
                    fiber_g: grams("0.0"),
                    calories: Calories::from_str("120").unwrap(),
                    fat_g: grams("6.0"),
                    carbs_g: grams("9.0"),
                    alcohol_g: grams("0.0"),
                },
            ],
        };

        let md = render_food(&food, DEFAULT_COLUMNS).unwrap();
        assert!(md.starts_with("\u{1b}[1;36mOatmeal (2 servings)\u{1b}[0m\n"));
        assert!(
            md.contains("  Oats        100g         200     30.0g    4.0g       10.0g      5.0g")
        );
        assert!(
            md.contains("  Milk        200ml        120      9.0g    6.0g        8.0g      0.0g")
        );
        assert!(md.contains("----------- ------  --------  --------  ------  ----------  --------"));
        assert!(md.contains(
            "\u{1b}[1;35mTotal                    320     39.0g   10.0g       18.0g      5.0g"
        ));
        assert!(md.contains(
            "\u{1b}[1;34mPer serving              160     19.5g    5.0g        9.0g      2.5g"
        ));
    }

    #[test]
    fn test_render_food_single_serving() {
        let food = food::Food {
            title: "Coffee".to_string(),
            servings: NonZeroU32::new(1).unwrap(),
            notes: String::new(),
            ingredients: vec![food::Ingredient {
                name: "Cold Brew".to_string(),
                quantity: None,
                protein_g: grams("0.0"),
                fiber_g: grams("0.0"),
                calories: Calories::ZERO,
                fat_g: grams("0.0"),
                carbs_g: grams("0.0"),
                alcohol_g: grams("0.0"),
            }],
        };

        let md = render_food(&food, DEFAULT_COLUMNS).unwrap();
        assert!(md.starts_with("\u{1b}[1;36mCoffee (1 serving)\u{1b}[0m\n"));
        assert!(md.contains("  Cold Brew"));
    }

    #[test]
    fn test_render_food_no_quantity() {
        let food = food::Food {
            title: "Test".to_string(),
            servings: NonZeroU32::new(1).unwrap(),
            notes: String::new(),
            ingredients: vec![food::Ingredient {
                name: "Secret Spice".to_string(),
                quantity: None,
                protein_g: grams("0.5"),
                fiber_g: grams("0.1"),
                calories: Calories::from_str("5").unwrap(),
                fat_g: grams("0.0"),
                carbs_g: grams("0.0"),
                alcohol_g: grams("0.0"),
            }],
        };

        let md = render_food(&food, DEFAULT_COLUMNS).unwrap();
        assert!(md.contains("  Secret Spice"));
        assert!(md.contains("  0.5g"));
        assert!(md.contains("  0.1g"));
    }

    #[test]
    fn test_render_food_column_subset() {
        let food = food_with_ingredient(1, ingredient("10.0", "5.0", 100, "4.0", "30.0", "0.0"));
        let md = render_food(&food, &[Column::Calories, Column::Fat]).unwrap();
        assert!(md.contains("Calories"));
        assert!(md.contains("Fat(g)"));
        assert!(md.contains("100"));
        assert!(md.contains("4.0g"));
        assert!(!md.contains("Carbs(g)"));
        assert!(!md.contains("Protein(g)"));
    }

    fn test_food(notes: &str) -> food::Food {
        let mut food =
            food_with_ingredient(1, ingredient("10.0", "5.0", 100, "4.0", "30.0", "0.0"));
        food.title = "Test".to_string();
        food.notes = notes.to_string();
        food
    }

    #[test]
    fn test_render_food_shows_notes_when_present() {
        let md = render_food(&test_food("Best eaten warm with salt."), DEFAULT_COLUMNS).unwrap();
        assert!(md.contains("Notes:"));
        assert!(md.contains("Best eaten warm with salt."));
    }

    #[test]
    fn test_render_food_hides_notes_when_empty() {
        let md = render_food(&test_food(""), DEFAULT_COLUMNS).unwrap();
        assert!(!md.contains("Notes:"));
    }

    #[test]
    fn test_render_food_hides_notes_when_whitespace() {
        let md = render_food(&test_food("   "), DEFAULT_COLUMNS).unwrap();
        assert!(!md.contains("Notes:"));
    }
}
